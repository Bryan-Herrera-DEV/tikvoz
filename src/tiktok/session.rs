//! Una sesión WebSocket contra el webcast de TikTok: conecta, entra a la sala,
//! mantiene el heartbeat, confirma (ack) los lotes y traduce mensajes a eventos.

use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::time::Duration;

use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use prost::Message;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http::HeaderValue};

use super::api::{SignedConnect, USER_AGENT};
use super::proto::{
    self, BaseMessage, ChatMessage, ControlMessage, FetchResult, GiftMessage, HeartBeat,
    ImEnterRoom, PushFrame, SocialMessage, User,
};
use super::{LiveError, LiveEvent};

const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(10);
/// Si no llega nada (ni datos ni heartbeat del server) en este tiempo, la conexión está muerta.
const READ_TIMEOUT: Duration = Duration::from_secs(45);

/// Parámetros fijos que manda el cliente web oficial al abrir el socket.
const WS_PARAMS: &[(&str, &str)] = &[
    ("aid", "1988"),
    ("app_language", "es-ES"),
    ("app_name", "tiktok_web"),
    ("browser_platform", "Win32"),
    ("browser_language", "es-ES"),
    ("browser_name", "Mozilla"),
    ("browser_version", "5.0%20(Windows%20NT%2010.0%3B%20Win64%3B%20x64)%20AppleWebKit%2F537.36%20(KHTML,%20like%20Gecko)%20Chrome%2F131.0.0.0%20Safari%2F537.36"),
    ("browser_online", "true"),
    ("cookie_enabled", "true"),
    ("tz_name", "America%2FMexico_City"),
    ("device_platform", "web"),
    ("identity", "audience"),
    ("live_id", "12"),
    ("sup_ws_ds_opt", "1"),
    ("update_version_code", "2.0.0"),
    ("version_code", "180800"),
    ("client_enter", "1"),
    ("ws_direct", "1"),
    ("did_rule", "3"),
    ("webcast_language", "es"),
    ("screen_height", "1080"),
    ("screen_width", "1920"),
    ("heartbeat_duration", "10000"),
    ("resp_content_type", "protobuf"),
    ("history_comment_count", "6"),
    ("last_rtt", "150"),
];

/// Cómo terminó la sesión (sin error).
pub enum SessionEnd {
    StreamEnded,
    SocketClosed,
}

pub async fn run(
    room_id: i64,
    signed: SignedConnect,
    emit: &(dyn Fn(LiveEvent) + Send + Sync),
) -> Result<SessionEnd, LiveError> {
    let SignedConnect { initial, cookies } = signed;
    let url = build_ws_url(&initial, room_id);

    let mut request = url
        .into_client_request()
        .map_err(|e| LiveError::Other(format!("URL de WebSocket inválida: {e}")))?;
    let headers = request.headers_mut();
    headers.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
    headers.insert(
        "Cookie",
        HeaderValue::from_str(&cookies).map_err(|e| LiveError::Other(e.to_string()))?,
    );
    headers.insert("Origin", HeaderValue::from_static("https://www.tiktok.com"));

    let (ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(map_ws_error)?;

    let heartbeat = response
        .headers()
        .get("Handshake-Options")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_ping_interval)
        .unwrap_or(DEFAULT_HEARTBEAT);

    log::info!("WebSocket conectado (sala {room_id}, heartbeat {heartbeat:?})");
    emit(LiveEvent::Connected { room_id });

    let mut dedup = Dedup::new(1024);
    // El primer lote viene del servidor de firmas: son comentarios recientes,
    // los mostramos pero marcados como historial (no se leen en voz alta).
    if dispatch(&initial.messages, true, &mut dedup, emit) {
        return Ok(SessionEnd::StreamEnded);
    }

    let (mut sink, mut stream) = ws.split();

    let enter = ImEnterRoom {
        room_id,
        live_id: 12,
        identity: "audience".into(),
        filter_welcome_msg: "0".into(),
        ..Default::default()
    };
    sink.send(frame("im_enter_room", enter.encode_to_vec(), 0))
        .await
        .map_err(map_ws_error)?;

    let mut seq = 1i64;
    let mut ticker = tokio::time::interval(heartbeat);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let hb = HeartBeat { room_id, send_packet_seq_id: seq };
                seq += 1;
                sink.send(frame("hb", hb.encode_to_vec(), 0)).await.map_err(map_ws_error)?;
            }
            next = tokio::time::timeout(READ_TIMEOUT, stream.next()) => {
                let msg = match next {
                    Err(_) => return Err(LiveError::Other("sin datos de TikTok, reconectando".into())),
                    Ok(None) => return Ok(SessionEnd::SocketClosed),
                    Ok(Some(m)) => m.map_err(map_ws_error)?,
                };
                let data = match msg {
                    tungstenite::Message::Binary(b) => b,
                    tungstenite::Message::Close(_) => return Ok(SessionEnd::SocketClosed),
                    _ => continue,
                };
                let Ok(push) = PushFrame::decode(data.as_ref()) else {
                    log::debug!("frame no decodificable ({} bytes)", data.len());
                    continue;
                };
                log::trace!("frame {} ({} bytes)", push.payload_type, push.payload.len());
                if push.payload_type != "msg" {
                    continue;
                }
                let result = match decode_fetch_result(&push) {
                    Ok(r) => r,
                    Err(e) => { log::warn!("lote inválido: {e}"); continue; }
                };
                if result.need_ack {
                    let ack_payload = if result.internal_ext.is_empty() {
                        b"-".to_vec()
                    } else {
                        result.internal_ext.clone().into_bytes()
                    };
                    sink.send(frame("ack", ack_payload, push.log_id)).await.map_err(map_ws_error)?;
                }
                if dispatch(&result.messages, false, &mut dedup, emit) {
                    let _ = sink.close().await;
                    return Ok(SessionEnd::StreamEnded);
                }
            }
        }
    }
}

fn frame(payload_type: &str, payload: Vec<u8>, log_id: i64) -> tungstenite::Message {
    let f = PushFrame {
        log_id,
        payload_encoding: "pb".into(),
        payload_type: payload_type.into(),
        payload,
        ..Default::default()
    };
    tungstenite::Message::Binary(f.encode_to_vec().into())
}

fn build_ws_url(initial: &FetchResult, room_id: i64) -> String {
    let mut params: Vec<String> = initial
        .route_params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", utf8_percent_encode(v, NON_ALPHANUMERIC)))
        .collect();
    params.extend(WS_PARAMS.iter().map(|(k, v)| format!("{k}={v}")));
    params.push(format!("room_id={room_id}"));
    params.push("compress=gzip".into());
    // El cliente oficial manda version_code dos veces; lo replicamos.
    params.push("version_code=270000".into());

    let sep = if initial.push_server.contains('?') { '&' } else { '?' };
    format!("{}{sep}{}", initial.push_server, params.join("&"))
}

fn parse_ping_interval(options: &str) -> Option<Duration> {
    options.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k.trim() == "ping-interval")
            .then(|| v.trim().parse::<f64>().ok())
            .flatten()
            .filter(|s| *s > 0.0 && *s < 120.0)
            .map(Duration::from_secs_f64)
    })
}

fn decode_fetch_result(push: &PushFrame) -> Result<FetchResult, String> {
    let gzip = push
        .headers
        .iter()
        .any(|h| h.key == "compress_type" && h.value == "gzip");
    if gzip {
        let mut buf = Vec::with_capacity(push.payload.len() * 4);
        GzDecoder::new(push.payload.as_slice())
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        FetchResult::decode(buf.as_slice()).map_err(|e| e.to_string())
    } else {
        FetchResult::decode(push.payload.as_slice()).map_err(|e| e.to_string())
    }
}

/// Traduce mensajes a eventos. Devuelve `true` si el live terminó.
fn dispatch(
    messages: &[BaseMessage],
    history: bool,
    dedup: &mut Dedup,
    emit: &(dyn Fn(LiveEvent) + Send + Sync),
) -> bool {
    for m in messages {
        if m.msg_id != 0 && !dedup.insert(m.msg_id) {
            continue;
        }
        let history = history || m.is_history;
        log::trace!("mensaje {} id={}", m.method, m.msg_id);
        match m.method.as_str() {
            "WebcastChatMessage" => {
                if let Ok(c) = ChatMessage::decode(m.payload.as_slice()) {
                    let (nickname, handle) = names(c.user);
                    emit(LiveEvent::Comment { nickname, handle, text: c.content, history });
                }
            }
            "WebcastGiftMessage" => {
                if let Ok(g) = GiftMessage::decode(m.payload.as_slice()) {
                    let gift = g.gift.unwrap_or_default();
                    // Los regalos "combo" llegan repetidos durante la racha: solo
                    // emitimos el final para no leer "rosa x1, rosa x2, rosa x3...".
                    if gift.r#type == 1 && g.repeat_end == 0 {
                        continue;
                    }
                    let (nickname, handle) = names(g.user);
                    emit(LiveEvent::Gift {
                        nickname,
                        handle,
                        gift: gift.name,
                        count: g.repeat_count.max(1) as u32,
                        diamonds: gift.diamond_count.max(0) as u32,
                        history,
                    });
                }
            }
            "WebcastSocialMessage" => {
                if let Ok(s) = SocialMessage::decode(m.payload.as_slice()) {
                    let (nickname, handle) = names(s.user);
                    match s.action {
                        1 => emit(LiveEvent::Follow { nickname, handle, history }),
                        3 => emit(LiveEvent::Share { nickname, handle, history }),
                        _ => {}
                    }
                }
            }
            "WebcastControlMessage" => {
                if let Ok(c) = ControlMessage::decode(m.payload.as_slice()) {
                    if c.action == proto::CONTROL_STREAM_ENDED
                        || c.action == proto::CONTROL_STREAM_SUSPENDED
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn names(user: Option<User>) -> (String, String) {
    let u = user.unwrap_or_default();
    (u.nickname, u.display_id)
}

fn map_ws_error(e: tungstenite::Error) -> LiveError {
    match e {
        tungstenite::Error::Http(resp) => {
            let reason = resp
                .headers()
                .get("Handshake-Msg")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("sin motivo")
                .to_string();
            LiveError::Other(format!("TikTok rechazó el WebSocket ({}): {reason}", resp.status()))
        }
        other => LiveError::Other(format!("WebSocket: {other}")),
    }
}

/// Conjunto acotado de IDs ya vistos (los lotes a veces se solapan).
struct Dedup {
    seen: HashSet<i64>,
    order: VecDeque<i64>,
    cap: usize,
}

impl Dedup {
    fn new(cap: usize) -> Self {
        Self { seen: HashSet::with_capacity(cap), order: VecDeque::with_capacity(cap), cap }
    }
    fn insert(&mut self, id: i64) -> bool {
        if !self.seen.insert(id) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_incluye_route_params_codificados() {
        let mut initial = FetchResult {
            push_server: "wss://webcast.example/ws".into(),
            ..Default::default()
        };
        initial.route_params.insert("ua".into(), "a b(c)".into());
        let url = build_ws_url(&initial, 42);
        assert!(url.starts_with("wss://webcast.example/ws?ua=a%20b%28c%29&aid=1988"));
        assert!(url.contains("&room_id=42&compress=gzip&version_code=270000"));
    }

    #[test]
    fn lee_ping_interval() {
        assert_eq!(parse_ping_interval("ping-interval=10; x=1"), Some(Duration::from_secs(10)));
        assert_eq!(parse_ping_interval("otra=1"), None);
    }

    #[test]
    fn dedup_descarta_repetidos_y_olvida_viejos() {
        let mut d = Dedup::new(2);
        assert!(d.insert(1));
        assert!(!d.insert(1));
        assert!(d.insert(2));
        assert!(d.insert(3)); // expulsa al 1
        assert!(d.insert(1));
    }

    #[test]
    fn regalo_combo_solo_al_final() {
        let gift = |end: i32| BaseMessage {
            method: "WebcastGiftMessage".into(),
            payload: GiftMessage {
                repeat_count: 5,
                repeat_end: end,
                gift: Some(proto::Gift { r#type: 1, name: "Rosa".into(), ..Default::default() }),
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        };
        let events = std::sync::Mutex::new(Vec::new());
        let emit = |e: LiveEvent| events.lock().unwrap().push(e);
        dispatch(&[gift(0), gift(1)], false, &mut Dedup::new(8), &emit);
        let events = events.into_inner().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LiveEvent::Gift { count: 5, .. }));
    }
}
