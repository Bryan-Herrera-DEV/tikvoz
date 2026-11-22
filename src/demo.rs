//! Modo demo: un live simulado con usuarios ficticios de TikTok y Kick. Sirve
//! para probar voces, filtros y plantillas sin estar en vivo (y para capturas
//! sin datos reales).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::live::{Emit, LiveEvent, Platform, Status, Viewer};

/// (nick, handle, plataforma, seguidor, moderador, suscriptor)
const USERS: &[(&str, &str, Platform, bool, bool, bool)] = &[
    ("Luna ✨", "luna.dev", Platform::TikTok, true, false, false),
    (
        "Carlos",
        "carlitos_99",
        Platform::TikTok,
        false,
        false,
        false,
    ),
    ("Mod Ana", "ana_mod", Platform::TikTok, true, true, false),
    ("𝓢𝓸𝓯í𝓪", "sofi.art", Platform::TikTok, true, false, false),
    ("PedroGG", "pedrogg", Platform::Kick, false, false, true),
    ("ValeKick", "valekick", Platform::Kick, false, true, false),
    ("NeoByte", "neobyte", Platform::Kick, false, false, false),
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
    "eres un p1to jaja",
    "esto es 4buso",
    "qué a.buso de stream",
];

const GIFTS: &[(&str, u32)] = &[("Rosa", 1), ("TikTok", 1), ("Corazón", 5), ("León", 29999)];

/// Detiene el generador al soltarlo.
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
    let (nick, handle, platform, follower, moderator, subscriber) = USERS[i % USERS.len()];
    Viewer {
        follower: (platform == Platform::TikTok).then_some(follower),
        moderator,
        subscriber,
        ..Viewer::new(platform, nick, handle)
    }
}

fn run(emit: Emit, stop: Arc<AtomicBool>) {
    emit(LiveEvent::Status(
        Platform::Demo,
        Status::Live("Modo demo (live simulado)".into()),
    ));
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
        let ev = match tick % 11 {
            3 => {
                let (gift, diamonds) = GIFTS[rnd(GIFTS.len())];
                let user = Viewer {
                    platform: Platform::TikTok,
                    ..user
                };
                LiveEvent::Gift {
                    user,
                    gift: gift.into(),
                    count: 1 + rnd(5) as u32,
                    diamonds,
                    history: false,
                }
            }
            5 => LiveEvent::Follow {
                user: Viewer {
                    platform: Platform::TikTok,
                    ..user
                },
                history: false,
            },
            7 => LiveEvent::Join { user },
            8 => LiveEvent::Share {
                user: Viewer {
                    platform: Platform::TikTok,
                    ..user
                },
                history: false,
            },
            10 => {
                let user = Viewer {
                    platform: Platform::Kick,
                    ..user
                };
                if rnd(2) == 0 {
                    LiveEvent::Subscription {
                        user,
                        months: 1 + rnd(12) as u32,
                        gifted: 0,
                    }
                } else {
                    LiveEvent::Subscription {
                        user,
                        months: 0,
                        gifted: 1 + rnd(5) as u32,
                    }
                }
            }
            _ => LiveEvent::Comment {
                user,
                text: COMMENTS[rnd(COMMENTS.len())].into(),
                history: false,
            },
        };
        emit(ev);
        tick += 1;
        std::thread::sleep(Duration::from_millis(1200 + rnd(1500) as u64));
    }
    emit(LiveEvent::Status(Platform::Demo, Status::Idle));
}
