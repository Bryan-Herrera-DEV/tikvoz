//! Modo demo: un live simulado con usuarios ficticios. Sirve para probar voces,
//! filtros y plantillas sin estar en vivo (y para capturas sin datos reales).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::{Emit, LiveEvent, Status, Viewer};

const USERS: &[(&str, &str, bool, bool)] = &[
    // (nick, handle, seguidor, moderador)
    ("Luna ✨", "luna.dev", true, false),
    ("Carlos", "carlitos_99", false, false),
    ("Mod Ana", "ana_mod", true, true),
    ("𝓢𝓸𝓯í𝓪", "sofi.art", true, false),
    ("Pedro", "pedrogamer", false, false),
    ("Vale 🌸", "vale.rose", true, false),
];

const COMMENTS: &[&str] = &[
    "hola desde Quito!!",
    "jajajajajajajaja buenísimo",
    "qué voz es esa xd",
    "saludos a todos 🔥🔥🔥",
    "mira mi canal https://ejemplo.com/spam",
    "a qué hora es el próximo live?",
    "holaaaaaaaa",
    "!dado",
    "tqm, pq no saludas?",
    "primera vez por aquí, me encanta",
];

const GIFTS: &[(&str, u32)] = &[("Rosa", 1), ("TikTok", 1), ("Corazón", 5), ("León", 29999)];

/// Lanza el generador; se detiene al soltar el `DemoHandle`.
pub struct DemoHandle(Arc<AtomicBool>);

impl Drop for DemoHandle {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(emit: Emit) -> DemoHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let _ = std::thread::Builder::new()
        .name("demo".into())
        .stack_size(64 * 1024)
        .spawn(move || run(emit, flag));
    DemoHandle(stop)
}

fn viewer(i: usize) -> Viewer {
    let (nick, handle, follower, moderator) = USERS[i % USERS.len()];
    Viewer { nickname: nick.into(), handle: handle.into(), follower, moderator }
}

fn run(emit: Emit, stop: Arc<AtomicBool>) {
    emit(LiveEvent::Status(Status::Live("Modo demo (live simulado)".into())));
    // Generador pseudoaleatorio mínimo (xorshift): no vale la pena una dependencia.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut tick = 0usize;
    while !stop.load(Ordering::Relaxed) {
        let user = viewer(rnd(USERS.len()));
        let ev = match tick % 9 {
            3 => {
                let (gift, diamonds) = GIFTS[rnd(GIFTS.len())];
                LiveEvent::Gift { user, gift: gift.into(), count: 1 + rnd(5) as u32, diamonds, history: false }
            }
            5 => LiveEvent::Follow { user, history: false },
            7 => LiveEvent::Join { user },
            8 => LiveEvent::Share { user, history: false },
            _ => LiveEvent::Comment { user, text: COMMENTS[rnd(COMMENTS.len())].into(), history: false },
        };
        emit(ev);
        tick += 1;
        std::thread::sleep(Duration::from_millis(1200 + rnd(1500) as u64));
    }
}
