//! Tipos comunes a todas las plataformas (TikTok, Kick, demo).
//!
//! Cada cliente traduce su protocolo a `LiveEvent`; la app, los filtros y la voz
//! no saben de qué plataforma viene nada más allá del campo `platform`.

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    TikTok,
    Kick,
    Demo,
}

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Platform::TikTok => "TikTok",
            Platform::Kick => "Kick",
            Platform::Demo => "Demo",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone)]
pub enum LiveEvent {
    Status(Platform, Status),
    Comment { user: Viewer, text: String, history: bool },
    Gift { user: Viewer, gift: String, count: u32, diamonds: u32, history: bool },
    Follow { user: Viewer, history: bool },
    Share { user: Viewer, history: bool },
    Join { user: Viewer },
    /// Suscripción propia (`gifted == 0`) o suscripciones regaladas (`gifted > 0`).
    Subscription { user: Viewer, months: u32, gifted: u32 },
}

/// Quién hizo algo en el live.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewer {
    pub platform: Platform,
    pub nickname: String,
    /// El @handle o slug (sin @).
    pub handle: String,
    /// `Some(true)` si sigue al streamer; `None` si la plataforma no lo informa.
    pub follower: Option<bool>,
    pub moderator: bool,
    pub subscriber: bool,
}

impl Viewer {
    pub fn new(platform: Platform, nickname: impl Into<String>, handle: impl Into<String>) -> Self {
        Self {
            platform,
            nickname: nickname.into(),
            handle: handle.into(),
            follower: None,
            moderator: false,
            subscriber: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Connecting(String),
    Live(String),
    /// Esperando a que empiece el live (o reintentando tras un error).
    Waiting(String),
    Error(String),
}

impl Status {
    /// Hay una conexión activa o intentándose (el botón muestra "Desconectar").
    pub fn is_active(&self) -> bool {
        !matches!(self, Status::Idle | Status::Error(_))
    }
}

pub type Emit = Arc<dyn Fn(LiveEvent) + Send + Sync>;

/// Espera de reconexión con backoff exponencial (2 s → 60 s).
pub struct Backoff(std::time::Duration);

impl Default for Backoff {
    fn default() -> Self {
        Self(std::time::Duration::from_secs(2))
    }
}

impl Backoff {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn next_wait(&mut self) -> std::time::Duration {
        let wait = self.0;
        self.0 = (self.0 * 2).min(std::time::Duration::from_secs(60));
        wait
    }
}
