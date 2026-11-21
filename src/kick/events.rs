//! Traducción de los mensajes de Centrifugo de Kick a `LiveEvent`.
//! Funciones puras: se prueban con mensajes reales capturados de kick.com.

use serde_json::Value;

use crate::live::{LiveEvent, Platform, Viewer};

/// Lo que puede traer un frame de Centrifugo.
#[derive(Debug, PartialEq)]
pub enum Frame {
    /// `{}`: ping del servidor; hay que responder `{}`.
    Ping,
    /// Respuesta a un comando nuestro (`id` > 0).
    Reply { id: u64, ttl: Option<u64>, error: Option<String> },
    /// Publicación en un canal suscrito.
    Push { event: String, data: Value, msg_id: Option<String> },
    /// El servidor pide desconectar.
    Disconnect(String),
    Other,
}

/// Un frame de texto puede traer varios JSON separados por salto de línea.
pub fn parse_frames(text: &str) -> Vec<Frame> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<Value>(l).map(parse_one).unwrap_or(Frame::Other))
        .collect()
}

fn parse_one(v: Value) -> Frame {
    if v.as_object().is_some_and(|o| o.is_empty()) {
        return Frame::Ping;
    }
    if let Some(d) = v.get("disconnect") {
        let reason = d.get("reason").and_then(Value::as_str).unwrap_or("sin motivo");
        return Frame::Disconnect(reason.to_string());
    }
    if let Some(push) = v.get("push") {
        let Some(pdata) = push.pointer("/pub/data") else { return Frame::Other };
        let event = pdata.get("event").and_then(Value::as_str).unwrap_or_default().to_string();
        // `data` llega como string JSON o como objeto, según el evento.
        let data = match pdata.get("data") {
            Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
            Some(other) => other.clone(),
            None => Value::Null,
        };
        let msg_id = data.get("id").and_then(Value::as_str).map(str::to_string);
        return Frame::Push { event, data, msg_id };
    }
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        let error = v
            .get("error")
            .map(|e| e.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
        let ttl = v
            .pointer("/connect/ttl")
            .or_else(|| v.pointer("/refresh/ttl"))
            .and_then(Value::as_u64);
        return Frame::Reply { id, ttl, error };
    }
    Frame::Other
}

/// Último segmento del nombre del evento: `App\Events\ChatMessageEvent` → `ChatMessageEvent`.
fn short(event: &str) -> &str {
    event.rsplit(['\\', '.']).next().unwrap_or(event)
}

fn str_at<'a>(v: &'a Value, paths: &[&str]) -> Option<&'a str> {
    paths
        .iter()
        .find_map(|p| v.pointer(p).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
}

fn viewer_from_sender(sender: &Value) -> Viewer {
    let username = str_at(sender, &["/username"]).unwrap_or_default();
    let slug = str_at(sender, &["/slug"]).unwrap_or(username);
    let mut v = Viewer::new(Platform::Kick, username, slug);
    if let Some(badges) = sender.pointer("/identity/badges").and_then(Value::as_array) {
        for b in badges {
            match b.get("type").and_then(Value::as_str).unwrap_or_default() {
                "moderator" | "broadcaster" => v.moderator = true,
                "subscriber" | "founder" | "og" => v.subscriber = true,
                _ => {}
            }
        }
    }
    v
}

fn named(username: &str) -> Viewer {
    Viewer::new(Platform::Kick, username, username.to_lowercase())
}

/// Quita los emotes `[emote:123:Nombre]` (la voz los leería como texto raro).
pub fn strip_emotes(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(i) = rest.find("[emote:") {
        out.push_str(&rest[..i]);
        match rest[i..].find(']') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Resultado de un evento de Kick.
pub enum Translated {
    Event(LiveEvent),
    StreamEnded,
    StreamStarted,
}

pub fn translate(event: &str, data: &Value) -> Option<Translated> {
    let ev = match short(event) {
        "ChatMessageEvent" => {
            let content = strip_emotes(data.get("content").and_then(Value::as_str).unwrap_or_default());
            if content.is_empty() {
                return None; // mensaje solo de emotes
            }
            let user = viewer_from_sender(data.get("sender")?);
            LiveEvent::Comment { user, text: content, history: false }
        }
        "SubscriptionEvent" => {
            let name = str_at(data, &["/username", "/user/username"])?;
            let months = data.get("months").and_then(Value::as_u64).unwrap_or(1) as u32;
            LiveEvent::Subscription { user: named(name), months, gifted: 0 }
        }
        "GiftedSubscriptionsEvent" => {
            let name = str_at(data, &["/gifter_username", "/gifted_by", "/gifter/username"]).unwrap_or("Anónimo");
            let count = ["gifted_usernames", "recipients"]
                .iter()
                .find_map(|k| data.get(*k).and_then(Value::as_array).map(Vec::len))
                .or_else(|| data.get("gift_count").and_then(Value::as_u64).map(|n| n as usize))
                .unwrap_or(1) as u32;
            LiveEvent::Subscription { user: named(name), months: 0, gifted: count.max(1) }
        }
        "KicksGifted" | "KicksGiftedEvent" => {
            let name = str_at(data, &["/sender/username", "/username"])?;
            let gift = str_at(data, &["/gift/name"]).unwrap_or("Kicks").to_string();
            let amount = data.pointer("/gift/amount").and_then(Value::as_u64).unwrap_or(0) as u32;
            LiveEvent::Gift { user: named(name), gift, count: 1, diamonds: amount, history: false }
        }
        "StopStreamBroadcast" => return Some(Translated::StreamEnded),
        "StreamerIsLive" => return Some(Translated::StreamStarted),
        _ => return None,
    };
    Some(Translated::Event(ev))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame real capturado de kick.com (noviembre 2026), datos anonimizados.
    const CHAT: &str = r##"{"push":{"channel":"chatrooms.668.v2","pub":{"data":{"event":"App\\Events\\ChatMessageEvent","data":"{\"id\":\"b329b178\",\"chatroom_id\":668,\"content\":\"bad ending [emote:4148081:Sadge]\",\"type\":\"message\",\"created_at\":\"2026-11-22T20:44:06+00:00\",\"sender\":{\"id\":1,\"username\":\"Usuario_X\",\"slug\":\"usuario-x\",\"identity\":{\"color\":\"#FFD899\",\"badges\":[{\"type\":\"subscriber\",\"text\":\"Subscriber\",\"count\":3},{\"type\":\"moderator\",\"text\":\"Moderator\"}]}}}"}}}}"##;

    #[test]
    fn parsea_chat_real() {
        let frames = parse_frames(CHAT);
        let Frame::Push { event, data, msg_id } = &frames[0] else { panic!("{frames:?}") };
        assert_eq!(msg_id.as_deref(), Some("b329b178"));
        let Some(Translated::Event(LiveEvent::Comment { user, text, .. })) = translate(event, data) else {
            panic!("no es comentario")
        };
        assert_eq!(text, "bad ending");
        assert_eq!(user.nickname, "Usuario_X");
        assert_eq!(user.handle, "usuario-x");
        assert!(user.moderator && user.subscriber);
        assert_eq!(user.platform, Platform::Kick);
    }

    #[test]
    fn ping_respuestas_y_lineas_multiples() {
        let frames = parse_frames("{}\n{\"id\":1,\"connect\":{\"client\":\"x\",\"ttl\":2699,\"ping\":25,\"pong\":true}}\n{\"id\":5,\"subscribe\":{}}");
        assert_eq!(frames[0], Frame::Ping);
        assert_eq!(frames[1], Frame::Reply { id: 1, ttl: Some(2699), error: None });
        assert_eq!(frames[2], Frame::Reply { id: 5, ttl: None, error: None });
        let err = parse_frames(r#"{"id":2,"error":{"code":103,"message":"permission denied"}}"#);
        assert_eq!(err[0], Frame::Reply { id: 2, ttl: None, error: Some("permission denied".into()) });
    }

    #[test]
    fn suscripciones_y_regalos() {
        let sub = translate("App\\Events\\SubscriptionEvent", &serde_json::json!({"username":"Ana","months":3}));
        assert!(matches!(sub, Some(Translated::Event(LiveEvent::Subscription { months: 3, gifted: 0, .. }))));
        let gifted = translate(
            "App\\Events\\GiftedSubscriptionsEvent",
            &serde_json::json!({"gifter_username":"Luis","gifted_usernames":["a","b","c"]}),
        );
        assert!(matches!(gifted, Some(Translated::Event(LiveEvent::Subscription { gifted: 3, .. }))));
        let solo_emotes = translate(
            "App\\Events\\ChatMessageEvent",
            &serde_json::json!({"content":"[emote:1:KEKW] [emote:2:LUL]","sender":{"username":"a"}}),
        );
        assert!(solo_emotes.is_none());
    }

    #[test]
    fn quita_emotes() {
        assert_eq!(strip_emotes("hola [emote:37226:KEKW] amigos"), "hola amigos");
        assert_eq!(strip_emotes("[emote:roto"), "");
    }
}
