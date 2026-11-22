//! Motor de voz en su propio hilo con una cola propia.
//!
//! El crate `tts` ya encola internamente, pero esa cola no se puede inspeccionar
//! ni recortar. Para mantener la latencia baja en un live con mucho chat usamos
//! nuestra propia `VecDeque` con tope: si se llena, se descartan los comentarios
//! más viejos (lo que importa es leer lo reciente, no ir 3 minutos tarde).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tts::Tts;

/// Cada cuánto el hilo revisa si terminó de hablar. 15 ms es imperceptible y
/// casi no consume CPU.
const POLL: Duration = Duration::from_millis(15);

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
    pub language: String,
}

#[derive(Debug, Clone)]
pub struct EngineInfo {
    pub voices: Vec<VoiceInfo>,
    pub current_voice: Option<String>,
    pub min_rate: f32,
    pub max_rate: f32,
    pub normal_rate: f32,
    pub min_volume: f32,
    pub max_volume: f32,
}

/// Un mensaje a leer.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub text: String,
    /// Voz específica para este mensaje (p. ej. voz por usuario); `None` = la principal.
    pub voice: Option<String>,
    /// Se adelanta a los mensajes normales (regalos, VIP, moderadores).
    pub priority: bool,
}

impl Utterance {
    pub fn plain(text: String) -> Self {
        Self {
            text,
            voice: None,
            priority: false,
        }
    }
}

enum Cmd {
    Enqueue(Utterance),
    SayNow(String),
    Skip,
    Clear,
    SetVoice(String),
    SetRate(f32),
    SetVolume(f32),
    SetPaused(bool),
    SetMaxQueue(usize),
}

/// Estado que la UI puede leer sin bloquear al hilo de voz.
#[derive(Default)]
pub struct Shared {
    queued: AtomicUsize,
    dropped: AtomicU64,
    spoken: AtomicU64,
    current: Mutex<Option<String>>,
}

impl Shared {
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub fn spoken(&self) -> u64 {
        self.spoken.load(Ordering::Relaxed)
    }
    pub fn current(&self) -> Option<String> {
        self.current.lock().ok().and_then(|c| c.clone())
    }
}

/// Handle barato de clonar para mandar órdenes al hilo de voz.
#[derive(Clone)]
pub struct Speaker {
    tx: Sender<Cmd>,
    pub shared: Arc<Shared>,
}

impl Speaker {
    /// Arranca el hilo de voz. Devuelve el handle y la info del motor (voces, rangos).
    pub fn spawn(
        max_queue: usize,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<(Self, EngineInfo)> {
        let (tx, rx) = mpsc::channel();
        let (info_tx, info_rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let worker_shared = shared.clone();

        thread::Builder::new()
            .name("voz".into())
            .spawn(move || match init_engine() {
                Ok((tts, info)) => {
                    let _ = info_tx.send(Ok(info));
                    Worker::new(tts, rx, worker_shared, max_queue, Box::new(on_change)).run();
                }
                Err(e) => {
                    let _ = info_tx.send(Err(e));
                }
            })?;

        let info = info_rx
            .recv()
            .map_err(|_| anyhow!("el hilo de voz terminó antes de iniciar"))??;
        Ok((Self { tx, shared }, info))
    }

    fn send(&self, cmd: Cmd) {
        // Si el hilo murió no hay nada útil que hacer desde la UI.
        let _ = self.tx.send(cmd);
    }

    pub fn enqueue(&self, u: Utterance) {
        self.send(Cmd::Enqueue(u));
    }
    /// Corta lo que está sonando y lee esto ya (para probar la voz).
    pub fn say_now(&self, text: String) {
        self.send(Cmd::SayNow(text));
    }
    pub fn skip(&self) {
        self.send(Cmd::Skip);
    }
    pub fn clear(&self) {
        self.send(Cmd::Clear);
    }
    pub fn set_voice(&self, id: String) {
        self.send(Cmd::SetVoice(id));
    }
    pub fn set_rate(&self, rate: f32) {
        self.send(Cmd::SetRate(rate));
    }
    pub fn set_volume(&self, volume: f32) {
        self.send(Cmd::SetVolume(volume));
    }
    pub fn set_paused(&self, paused: bool) {
        self.send(Cmd::SetPaused(paused));
    }
    pub fn set_max_queue(&self, max: usize) {
        self.send(Cmd::SetMaxQueue(max));
    }
}

fn init_engine() -> Result<(Tts, EngineInfo)> {
    let tts = Tts::default().map_err(|e| anyhow!("no se pudo iniciar el motor de voz: {e}"))?;
    let voices = tts
        .voices()
        .unwrap_or_default()
        .into_iter()
        .map(|v| VoiceInfo {
            id: v.id(),
            name: v.name(),
            language: v.language().to_string(),
        })
        .collect();
    let current_voice = tts.voice().ok().flatten().map(|v| v.id());
    let info = EngineInfo {
        voices,
        current_voice,
        min_rate: tts.min_rate(),
        max_rate: tts.max_rate(),
        normal_rate: tts.normal_rate(),
        min_volume: tts.min_volume(),
        max_volume: tts.max_volume(),
    };
    Ok((tts, info))
}

struct Worker {
    tts: Tts,
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    queue: VecDeque<Utterance>,
    /// Mensajes prioritarios: se leen antes que `queue`.
    priority: VecDeque<Utterance>,
    max_queue: usize,
    /// Voz principal elegida en la UI y la que está cargada ahora en el motor.
    main_voice: Option<String>,
    loaded_voice: Option<String>,
    paused: bool,
    speaking: bool,
    on_change: Box<dyn Fn() + Send>,
}

impl Worker {
    fn new(
        tts: Tts,
        rx: Receiver<Cmd>,
        shared: Arc<Shared>,
        max_queue: usize,
        on_change: Box<dyn Fn() + Send>,
    ) -> Self {
        Self {
            tts,
            rx,
            shared,
            queue: VecDeque::new(),
            priority: VecDeque::new(),
            max_queue: max_queue.max(1),
            main_voice: None,
            loaded_voice: None,
            paused: false,
            speaking: false,
            on_change,
        }
    }

    fn run(mut self) {
        loop {
            match self.rx.recv_timeout(POLL) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // Procesa en ráfaga todo lo pendiente antes de decidir qué hablar.
            while let Ok(cmd) = self.rx.try_recv() {
                self.handle(cmd);
            }
            self.tick();
        }
        let _ = self.tts.stop();
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Enqueue(u) => {
                if u.priority {
                    self.priority.push_back(u);
                } else {
                    self.queue.push_back(u);
                }
                self.trim_queue();
            }
            Cmd::SayNow(text) => {
                self.stop_current();
                self.speak(Utterance::plain(text));
            }
            Cmd::Skip => self.stop_current(),
            Cmd::Clear => {
                self.queue.clear();
                self.priority.clear();
                self.stop_current();
            }
            Cmd::SetVoice(id) => self.main_voice = Some(id),
            Cmd::SetRate(r) => {
                let _ = self
                    .tts
                    .set_rate(r.clamp(self.tts.min_rate(), self.tts.max_rate()));
            }
            Cmd::SetVolume(v) => {
                let _ = self
                    .tts
                    .set_volume(v.clamp(self.tts.min_volume(), self.tts.max_volume()));
            }
            Cmd::SetPaused(p) => {
                self.paused = p;
                if p {
                    self.stop_current();
                }
            }
            Cmd::SetMaxQueue(m) => {
                self.max_queue = m.max(1);
                self.trim_queue();
            }
        }
        self.publish();
    }

    fn tick(&mut self) {
        if self.speaking && !self.tts.is_speaking().unwrap_or(false) {
            self.speaking = false;
            self.shared.spoken.fetch_add(1, Ordering::Relaxed);
            self.set_current(None);
        }
        if !self.speaking && !self.paused {
            if let Some(next) = self.priority.pop_front().or_else(|| self.queue.pop_front()) {
                self.speak(next);
            }
        }
    }

    /// Carga la voz pedida solo si es distinta de la actual (cambiar de voz cuesta).
    fn ensure_voice(&mut self, wanted: Option<&String>) {
        let Some(id) = wanted.or(self.main_voice.as_ref()).cloned() else {
            return;
        };
        if self.loaded_voice.as_ref() == Some(&id) {
            return;
        }
        if let Some(v) = self
            .tts
            .voices()
            .ok()
            .and_then(|vs| vs.into_iter().find(|v| v.id() == id))
        {
            match self.tts.set_voice(&v) {
                Ok(()) => self.loaded_voice = Some(id),
                Err(e) => log::warn!("no se pudo cambiar la voz: {e}"),
            }
        }
    }

    fn speak(&mut self, u: Utterance) {
        self.ensure_voice(u.voice.as_ref());
        match self.tts.speak(u.text.as_str(), false) {
            Ok(_) => {
                self.speaking = true;
                self.set_current(Some(u.text));
            }
            Err(e) => log::warn!("error al hablar: {e}"),
        }
        self.publish();
    }

    fn stop_current(&mut self) {
        if self.speaking {
            let _ = self.tts.stop();
            self.speaking = false;
            self.set_current(None);
        }
    }

    /// Descarta lo más viejo cuando se supera el tope (primero lo normal).
    fn trim_queue(&mut self) {
        while self.queue.len() + self.priority.len() > self.max_queue {
            if self.queue.pop_front().is_none() {
                self.priority.pop_front();
            }
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn set_current(&self, text: Option<String>) {
        if let Ok(mut c) = self.shared.current.lock() {
            *c = text;
        }
        (self.on_change)();
    }

    fn publish(&self) {
        self.shared
            .queued
            .store(self.queue.len() + self.priority.len(), Ordering::Relaxed);
        (self.on_change)();
    }
}
