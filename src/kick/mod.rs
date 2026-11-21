//! Cliente de chat de Kick.
//!
//! Desde octubre de 2026 Kick usa **Centrifugo** (antes Pusher). Flujo anónimo,
//! el mismo que hace kick.com para un visitante sin sesión:
//! 1. `GET kick.com/api/v2/channels/<slug>` → `id` del canal y `chatroom.id`.
//! 2. `POST web.kick.com/api/v1/realtime/connection` → URL del WebSocket.
//! 3. `POST web.kick.com/api/v1/realtime/auth/connection` → token JWT de invitado.
//! 4. WebSocket (Origin `https://kick.com`), `connect` con el token y `subscribe`
//!    a `chatrooms.<id>.v2` y compañía. El servidor manda `{}` cada 25 s y hay que
//!    responder `{}`; el token se renueva con `refresh` antes de que expire.

mod events;
mod session;

use std::fmt;
use std::time::Duration;

use reqwest::{header, Client};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::live::{Backoff, Emit, LiveEvent, Platform, Status};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const ORIGIN: &str = "https://kick.com";
const CHANNEL_API: &str = "https://kick.com/api/v2/channels";
const REALTIME_CONFIG_URL: &str = "https://web.kick.com/api/v1/realtime/connection";
const REALTIME_AUTH_URL: &str = "https://web.kick.com/api/v1/realtime/auth/connection";
/// Si la API de configuración no devuelve URL, usamos la que usa kick.com.
const DEFAULT_WS_URL: &str = "wss://realtime.us-west-2.platform.kick.com/connection/websocket";

#[derive(Debug, Clone)]
pub enum KickError {
    ChannelNotFound,
    Network(String),
    Protocol(String),
}

impl fmt::Display for KickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KickError::ChannelNotFound => write!(f, "el canal no existe"),
            KickError::Network(m) => write!(f, "error de red: {m}"),
            KickError::Protocol(m) => write!(f, "{m}"),
        }
    }
}

impl KickError {
    fn net(e: reqwest::Error) -> Self {
        KickError::Network(e.without_url().to_string())
    }
}

/// Datos del canal necesarios para suscribirse.
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub slug: String,
    pub channel_id: u64,
    pub chatroom_id: u64,
    pub live: bool,
}

#[derive(Deserialize)]
struct ChannelResponse {
    id: u64,
    slug: String,
    chatroom: Chatroom,
    livestream: Option<Value>,
}
#[derive(Deserialize)]
struct Chatroom {
    id: u64,
}

pub(crate) struct Api {
    http: Client,
    client_id: String,
}

impl Api {
    fn new() -> reqwest::Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(8))
            .build()?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let client_id = format!("tikvoz-{:x}-{nanos:x}", std::process::id());
        Ok(Self { http, client_id })
    }

    async fn channel(&self, slug: &str) -> Result<ChannelInfo, KickError> {
        let resp = self
            .http
            .get(format!("{CHANNEL_API}/{slug}"))
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(KickError::net)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(KickError::ChannelNotFound);
        }
        if !resp.status().is_success() {
            return Err(KickError::Protocol(format!("Kick respondió {} al buscar el canal", resp.status())));
        }
        let c: ChannelResponse = resp
            .json()
            .await
            .map_err(|e| KickError::Protocol(format!("respuesta inesperada de Kick: {e}")))?;
        Ok(ChannelInfo {
            slug: c.slug,
            channel_id: c.id,
            chatroom_id: c.chatroom.id,
            live: c.livestream.is_some_and(|l| !l.is_null()),
        })
    }

    async fn post(&self, url: &str, body: Value) -> Result<Value, KickError> {
        let resp = self
            .http
            .post(url)
            .header(header::ACCEPT, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header(header::REFERER, "https://kick.com/")
            .header("x-app-platform", "web")
            .json(&body)
            .send()
            .await
            .map_err(KickError::net)?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(KickError::Protocol(format!("Kick respondió {status} en {url}")));
        }
        Ok(v)
    }

    /// URL del WebSocket de Centrifugo.
    async fn ws_url(&self) -> String {
        let body = json!({
            "client": { "id": self.client_id, "type": "web" },
            "capabilities": { "accepted_providers": [{ "provider": "centrifugo" }] }
        });
        match self.post(REALTIME_CONFIG_URL, body).await {
            Ok(v) => v
                .pointer("/data/connections/0/credentials/url")
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("wss://"))
                .unwrap_or(DEFAULT_WS_URL)
                .to_string(),
            Err(e) => {
                log::debug!("config realtime de Kick falló ({e}); uso URL por defecto");
                DEFAULT_WS_URL.to_string()
            }
        }
    }

    /// Token JWT de invitado.
    async fn token(&self) -> Result<String, KickError> {
        let v = self
            .post(REALTIME_AUTH_URL, json!({ "client_id": self.client_id }))
            .await?;
        v.pointer("/data/token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| KickError::Protocol("Kick no devolvió token de chat".into()))
    }
}

enum Cmd {
    Connect(String),
    Disconnect,
}

#[derive(Clone)]
pub struct KickClient {
    tx: UnboundedSender<Cmd>,
}

impl KickClient {
    pub fn spawn(emit: Emit) -> std::io::Result<Self> {
        let (tx, rx) = unbounded_channel();
        std::thread::Builder::new().name("kick".into()).spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime tokio");
            rt.block_on(supervisor(rx, emit));
        })?;
        Ok(Self { tx })
    }

    pub fn connect(&self, channel: &str) {
        let _ = self.tx.send(Cmd::Connect(normalize_slug(channel)));
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(Cmd::Disconnect);
    }
}

/// Acepta "canal", "@canal" o "https://kick.com/canal".
pub fn normalize_slug(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let s = s.rsplit('/').next().unwrap_or(s);
    s.trim_start_matches('@').to_lowercase()
}

async fn supervisor(mut rx: UnboundedReceiver<Cmd>, emit: Emit) {
    let mut task: Option<JoinHandle<()>> = None;
    while let Some(cmd) = rx.recv().await {
        if let Some(t) = task.take() {
            t.abort();
        }
        match cmd {
            Cmd::Connect(slug) => task = Some(tokio::spawn(keep_connected(slug, emit.clone()))),
            Cmd::Disconnect => emit(LiveEvent::Status(Platform::Kick, Status::Idle)),
        }
    }
}

async fn keep_connected(slug: String, emit: Emit) {
    let status = |s: Status| emit(LiveEvent::Status(Platform::Kick, s));
    let api = match Api::new() {
        Ok(a) => a,
        Err(e) => return status(Status::Error(format!("no se pudo crear cliente HTTP: {e}"))),
    };
    let mut backoff = Backoff::default();
    loop {
        status(Status::Connecting(format!("Conectando con kick.com/{slug}…")));
        let result = async {
            let info = api.channel(&slug).await?;
            let url = api.ws_url().await;
            let token = api.token().await?;
            session::run(&api, &info, &url, token, emit.as_ref(), &mut backoff).await
        }
        .await;
        let wait = match result {
            Ok(()) => {
                status(Status::Waiting("Kick cerró la conexión, reconectando…".into()));
                Duration::from_secs(1)
            }
            Err(KickError::ChannelNotFound) => {
                return status(Status::Error(format!("kick.com/{slug}: el canal no existe")));
            }
            Err(e) => {
                let wait = backoff.next_wait();
                log::warn!("Kick: {e}");
                status(Status::Waiting(format!("{e} — reintento en {} s", wait.as_secs())));
                wait
            }
        };
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_slug;

    #[test]
    fn acepta_slug_url_o_arroba() {
        assert_eq!(normalize_slug("XQC"), "xqc");
        assert_eq!(normalize_slug("@xqc"), "xqc");
        assert_eq!(normalize_slug("https://kick.com/xqc/"), "xqc");
    }
}
