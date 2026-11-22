//! Cliente de TikTok LIVE.
//!
//! Corre en un hilo con un runtime tokio de un solo hilo (suficiente para un
//! socket y es lo más liviano). La UI le manda órdenes (`connect`/`disconnect`)
//! y recibe `LiveEvent`s por un canal estándar.

mod api;
mod proto;
mod session;

use std::fmt;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::live::{Backoff, Emit, LiveEvent, Platform, Status};

#[derive(Debug, Clone)]
pub enum LiveError {
    UserNotFound,
    Offline,
    RateLimited(String),
    Network(String),
    Other(String),
}

impl LiveError {
    fn net(e: reqwest::Error) -> Self {
        LiveError::Network(e.without_url().to_string())
    }
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiveError::UserNotFound => write!(f, "el usuario no existe o no puede hacer lives"),
            LiveError::Offline => write!(f, "el usuario no está en vivo"),
            LiveError::RateLimited(m) => write!(f, "límite del servidor de firmas: {m}"),
            LiveError::Network(m) => write!(f, "error de red: {m}"),
            LiveError::Other(m) => write!(f, "{m}"),
        }
    }
}

enum Cmd {
    Connect {
        username: String,
        api_key: Option<String>,
    },
    Disconnect,
}

#[derive(Clone)]
pub struct LiveClient {
    tx: UnboundedSender<Cmd>,
}

impl LiveClient {
    pub fn spawn(emit: Emit) -> std::io::Result<Self> {
        let (tx, rx) = unbounded_channel();
        std::thread::Builder::new()
            .name("tiktok".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime tokio");
                rt.block_on(supervisor(rx, emit));
            })?;
        Ok(Self { tx })
    }

    pub fn connect(&self, username: &str, api_key: Option<String>) {
        let username = username.trim().trim_start_matches('@').to_string();
        let _ = self.tx.send(Cmd::Connect { username, api_key });
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(Cmd::Disconnect);
    }
}

async fn supervisor(mut rx: UnboundedReceiver<Cmd>, emit: Emit) {
    let mut task: Option<JoinHandle<()>> = None;
    while let Some(cmd) = rx.recv().await {
        if let Some(t) = task.take() {
            t.abort();
        }
        match cmd {
            Cmd::Connect { username, api_key } => {
                task = Some(tokio::spawn(keep_connected(
                    username,
                    api_key,
                    emit.clone(),
                )));
            }
            Cmd::Disconnect => emit(LiveEvent::Status(Platform::TikTok, Status::Idle)),
        }
    }
}

/// Mantiene la conexión viva: si el live no ha empezado espera, si se cae reconecta
/// con backoff exponencial.
async fn keep_connected(username: String, api_key: Option<String>, emit: Emit) {
    let status = |s: Status| emit(LiveEvent::Status(Platform::TikTok, s));
    let http = match api::http_client() {
        Ok(c) => c,
        Err(e) => return status(Status::Error(format!("no se pudo crear cliente HTTP: {e}"))),
    };
    let mut backoff = Backoff::default();

    loop {
        status(Status::Connecting(format!("Conectando con @{username}…")));
        let result = async {
            let room_id = api::fetch_room_id(&http, &username).await?;
            let signed = api::fetch_signed_ws(&http, room_id, api_key.as_deref()).await?;
            status(Status::Live(format!("En vivo: @{username}")));
            session::run(room_id, signed, emit.as_ref()).await
        }
        .await;

        let wait = match result {
            Ok(session::SessionEnd::StreamEnded) => {
                backoff.reset();
                status(Status::Waiting(format!(
                    "@{username} terminó el live; esperando que vuelva…"
                )));
                Duration::from_secs(30)
            }
            Ok(session::SessionEnd::SocketClosed) => {
                backoff.reset();
                status(Status::Waiting(
                    "Conexión cerrada por TikTok, reconectando…".into(),
                ));
                Duration::from_secs(1)
            }
            Err(LiveError::UserNotFound) => {
                return status(Status::Error(format!(
                    "@{username}: {}",
                    LiveError::UserNotFound
                )));
            }
            Err(LiveError::Offline) => {
                backoff.reset();
                status(Status::Waiting(format!(
                    "@{username} no está en vivo; reintento en 30 s"
                )));
                Duration::from_secs(30)
            }
            Err(e) => {
                let wait = backoff.next_wait();
                log::warn!("{e}");
                status(Status::Waiting(format!(
                    "{e} — reintento en {} s",
                    wait.as_secs()
                )));
                wait
            }
        };
        tokio::time::sleep(wait).await;
    }
}
