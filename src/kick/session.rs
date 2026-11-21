//! Sesión WebSocket con Centrifugo (protocolo JSON).

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http::HeaderValue};

use super::events::{parse_frames, translate, Frame, Translated};
use super::{Api, ChannelInfo, KickError, ORIGIN, USER_AGENT};
use crate::live::{Backoff, LiveEvent, Platform, Status};

/// El servidor hace ping cada 25 s; si en 60 s no llega nada, la conexión murió.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_ID: u64 = 1;

pub async fn run(
    api: &Api,
    info: &ChannelInfo,
    url: &str,
    token: String,
    emit: &(dyn Fn(LiveEvent) + Send + Sync),
    backoff: &mut Backoff,
) -> Result<(), KickError> {
    let status = |s: Status| emit(LiveEvent::Status(Platform::Kick, s));
    let mut request = url
        .into_client_request()
        .map_err(|e| KickError::Protocol(format!("URL de WebSocket inválida: {e}")))?;
    let headers = request.headers_mut();
    // Centrifugo de Kick rechaza (403) cualquier Origin que no sea kick.com.
    headers.insert("Origin", HeaderValue::from_static(ORIGIN));
    headers.insert("User-Agent", HeaderValue::from_static(USER_AGENT));

    let (ws, _) = tokio_tungstenite::connect_async(request).await.map_err(ws_err)?;
    let (mut sink, mut stream) = ws.split();

    let send = |v: serde_json::Value| tungstenite::Message::Text(v.to_string().into());
    sink.send(send(json!({ "id": CONNECT_ID, "connect": { "token": token, "name": "js" } })))
        .await
        .map_err(ws_err)?;

    let channels = [
        format!("chatrooms.{}.v2", info.chatroom_id),
        format!("chatrooms.{}", info.chatroom_id),
        format!("chatroom_{}", info.chatroom_id),
        format!("channel.{}", info.channel_id),
    ];
    let mut next_id = CONNECT_ID + 1;
    let mut connected = false;
    let mut refresh_at: Option<tokio::time::Instant> = None;
    let mut seen = Dedup::new(512);
    let live_text = |live: bool| {
        if live {
            format!("En vivo: kick.com/{}", info.slug)
        } else {
            format!("Chat conectado: kick.com/{} (offline)", info.slug)
        }
    };

    loop {
        let refresh_sleep = async {
            match refresh_at {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = refresh_sleep => {
                // El token de invitado expira (~45 min): pedimos otro y lo renovamos en caliente.
                refresh_at = None;
                let token = api.token().await?;
                sink.send(send(json!({ "id": next_id, "refresh": { "token": token } }))).await.map_err(ws_err)?;
                next_id += 1;
            }
            next = tokio::time::timeout(READ_TIMEOUT, stream.next()) => {
                let msg = match next {
                    Err(_) => return Err(KickError::Protocol("sin datos de Kick, reconectando".into())),
                    Ok(None) => return Ok(()),
                    Ok(Some(m)) => m.map_err(ws_err)?,
                };
                let text = match msg {
                    tungstenite::Message::Text(t) => t,
                    tungstenite::Message::Close(_) => return Ok(()),
                    _ => continue,
                };
                for frame in parse_frames(&text) {
                    match frame {
                        Frame::Ping => sink.send(send(json!({}))).await.map_err(ws_err)?,
                        Frame::Reply { error: Some(e), id, .. } => {
                            return Err(KickError::Protocol(format!("Kick rechazó la conexión (#{id}): {e}")));
                        }
                        Frame::Reply { id: CONNECT_ID, ttl, .. } if !connected => {
                            connected = true;
                            backoff.reset();
                            for ch in &channels {
                                sink.send(send(json!({ "id": next_id, "subscribe": { "channel": ch } })))
                                    .await
                                    .map_err(ws_err)?;
                                next_id += 1;
                            }
                            status(Status::Live(live_text(info.live)));
                            refresh_at = refresh_deadline(ttl);
                        }
                        Frame::Reply { ttl, .. } => {
                            if ttl.is_some() {
                                refresh_at = refresh_deadline(ttl);
                            }
                        }
                        Frame::Push { event, data, msg_id } => {
                            // Kick publica el mismo chat en varios canales: deduplicamos por id.
                            if msg_id.as_ref().is_some_and(|id| !seen.insert(id)) {
                                continue;
                            }
                            match translate(&event, &data) {
                                Some(Translated::Event(e)) => emit(e),
                                Some(Translated::StreamEnded) => status(Status::Live(live_text(false))),
                                Some(Translated::StreamStarted) => status(Status::Live(live_text(true))),
                                None => log::trace!("Kick: evento ignorado {event}"),
                            }
                        }
                        Frame::Disconnect(reason) => {
                            return Err(KickError::Protocol(format!("Kick desconectó: {reason}")));
                        }
                        Frame::Other => {}
                    }
                }
            }
        }
    }
}

/// Renovamos al 80 % de la vida del token.
fn refresh_deadline(ttl: Option<u64>) -> Option<tokio::time::Instant> {
    ttl.filter(|t| *t > 30)
        .map(|t| tokio::time::Instant::now() + Duration::from_secs(t * 8 / 10))
}

fn ws_err(e: tungstenite::Error) -> KickError {
    match e {
        tungstenite::Error::Http(resp) => {
            KickError::Protocol(format!("Kick rechazó el WebSocket ({})", resp.status()))
        }
        other => KickError::Network(format!("WebSocket: {other}")),
    }
}

struct Dedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl Dedup {
    fn new(cap: usize) -> Self {
        Self { seen: HashSet::new(), order: VecDeque::new(), cap }
    }
    fn insert(&mut self, id: &str) -> bool {
        if !self.seen.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}
