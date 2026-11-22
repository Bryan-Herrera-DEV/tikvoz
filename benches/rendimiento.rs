//! Benchmarks de las rutas que recorre cada mensaje del chat.
//!
//! Ejecutar:            cargo bench
//! Guardar referencia:  cargo bench -- --save-baseline main
//! Comparar contra ella: cargo bench -- --baseline main
//!
//! En los pull requests, `.github/workflows/benchmarks.yml` mide la rama base y
//! el PR en la misma máquina y publica la diferencia (ver CONTRIBUTING.md).
//!
//! Los grupos y nombres de cada benchmark son estables: cambiarlos rompe la
//! comparación con la rama base. Si agregas uno, usa un nombre nuevo.

use std::hint::black_box;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use flate2::write::GzEncoder;
use prost::Message;

use tikvoz::filter::{clean_comment, FilterConfig};
use tikvoz::kick::events::{parse_frames, translate};
use tikvoz::profanity::{Profanity, DEFAULT_BAD_WORDS};
use tikvoz::tiktok::decode_frame;
use tikvoz::tiktok::proto::{BaseMessage, ChatMessage, FetchResult, PushFrame, PushHeader, User};

/// Comentarios típicos de un live en español (la mayoría del tráfico real).
const CHAT_LIMPIO: &[&str] = &[
    "hola a todos desde Quito!!",
    "qué buen stream, saludos",
    "jajajajajajaja buenísimo",
    "a qué hora es el próximo live?",
    "cuánto cuesta el envío a Guayaquil",
    "primera vez por aquí, me encanta",
    "holaaaaaaa",
    "tqm, pq no saludas? xd",
    "mira mi canal https://ejemplo.com/canal",
    "saludos a todos 🔥🔥🔥",
    "Sofía dice que la música está muy alta",
    "precio del aire de 24000 btu",
    "el de 50 pulgadas es Samsung o LG?",
    "buenas noches desde Colombia",
    "qué juego es ese?",
    "dale like al live",
    "repito: cuánto cuesta?",
    "computadora gamer barata?",
    "la vergüenza de ese gol jaja",
    "nacional campeón",
];

/// Groserías ofuscadas: el caso más caro del detector.
const CHAT_OFUSCADO: &[&str] = &[
    "eres un p1to jaja",
    "esto es 4buso",
    "qué a.buso de stream",
    "pi to el que lo lea",
    "𝓹𝓲𝓽𝓸 total",
    "ᴘɪᴛᴏ",
    "oʇᴉd",
    "pu\u{200B}to",
    "v3rg4 qué partido",
    "hijo de puta el árbitro",
];

/// Frame real de Kick (capturado de kick.com, datos anonimizados).
const KICK_FRAME: &str = r##"{"push":{"channel":"chatrooms.668.v2","pub":{"data":{"event":"App\\Events\\ChatMessageEvent","data":"{\"id\":\"b329b178\",\"chatroom_id\":668,\"content\":\"bad ending [emote:4148081:Sadge]\",\"type\":\"message\",\"created_at\":\"2026-11-22T20:44:06+00:00\",\"sender\":{\"id\":1,\"username\":\"Usuario_X\",\"slug\":\"usuario-x\",\"identity\":{\"color\":\"#FFD899\",\"badges\":[{\"type\":\"subscriber\",\"text\":\"Subscriber\",\"count\":3},{\"type\":\"moderator\",\"text\":\"Moderator\"}]}}}"}}}}"##;

fn mensaje_largo() -> String {
    CHAT_LIMPIO.join(" ").chars().take(300).collect()
}

fn groserias(c: &mut Criterion) {
    let mut g = c.benchmark_group("groserias");
    g.bench_function("compilar_lista", |b| {
        b.iter(|| Profanity::with_options(black_box(DEFAULT_BAD_WORDS), "", &[]))
    });

    let p = Profanity::with_options(DEFAULT_BAD_WORDS, "", &[]);
    g.throughput(Throughput::Elements(CHAT_LIMPIO.len() as u64));
    g.bench_function("chat_limpio_20", |b| {
        b.iter(|| {
            for m in CHAT_LIMPIO {
                black_box(p.find(black_box(m)));
            }
        })
    });
    g.throughput(Throughput::Elements(CHAT_OFUSCADO.len() as u64));
    g.bench_function("chat_ofuscado_10", |b| {
        b.iter(|| {
            for m in CHAT_OFUSCADO {
                black_box(p.find(black_box(m)));
            }
        })
    });
    let largo = mensaje_largo();
    g.throughput(Throughput::Bytes(largo.len() as u64));
    g.bench_function("mensaje_largo_300", |b| {
        b.iter(|| black_box(p.find(black_box(&largo))))
    });
    g.finish();
}

fn limpieza(c: &mut Criterion) {
    let mut g = c.benchmark_group("limpieza");
    let cfg = FilterConfig::default();
    g.throughput(Throughput::Elements(CHAT_LIMPIO.len() as u64));
    g.bench_function("chat_limpio_20", |b| {
        b.iter(|| {
            for m in CHAT_LIMPIO {
                black_box(clean_comment(black_box(m), &["usuario", "Usuario"], &cfg));
            }
        })
    });
    g.finish();
}

fn kick(c: &mut Criterion) {
    let mut g = c.benchmark_group("kick");
    g.bench_function("frame_chat", |b| {
        b.iter(|| {
            for frame in parse_frames(black_box(KICK_FRAME)) {
                if let tikvoz::kick::events::Frame::Push { event, data, .. } = frame {
                    black_box(translate(&event, &data));
                }
            }
        })
    });
    g.finish();
}

/// Lote de TikTok tal como llega por el WebSocket: N comentarios en un
/// `FetchResult`, comprimido con gzip dentro de un `PushFrame`.
fn lote_tiktok(n: usize) -> Vec<u8> {
    let messages = CHAT_LIMPIO
        .iter()
        .cycle()
        .take(n)
        .enumerate()
        .map(|(i, text)| BaseMessage {
            method: "WebcastChatMessage".into(),
            msg_id: 7_710_000_000_000_000_000 + i as i64,
            payload: ChatMessage {
                user: Some(User {
                    id: i as i64,
                    nickname: format!("Usuario {i}"),
                    display_id: format!("usuario_{i}"),
                    ..Default::default()
                }),
                content: (*text).into(),
            }
            .encode_to_vec(),
            ..Default::default()
        })
        .collect();
    let result = FetchResult {
        messages,
        need_ack: true,
        internal_ext: "internal".into(),
        ..Default::default()
    };
    let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&result.encode_to_vec()).unwrap();
    PushFrame {
        log_id: 1,
        payload_type: "msg".into(),
        payload_encoding: "pb".into(),
        headers: vec![PushHeader {
            key: "compress_type".into(),
            value: "gzip".into(),
        }],
        payload: gz.finish().unwrap(),
        ..Default::default()
    }
    .encode_to_vec()
}

fn tiktok(c: &mut Criterion) {
    let mut g = c.benchmark_group("tiktok");
    let frame = lote_tiktok(20);
    let emitted = AtomicUsize::new(0);
    let emit = |_e| {
        emitted.fetch_add(1, Ordering::Relaxed);
    };
    g.throughput(Throughput::Elements(20));
    g.bench_function("lote_gzip_20_comentarios", |b| {
        b.iter(|| black_box(decode_frame(black_box(&frame), &emit)))
    });
    g.finish();
    assert!(
        emitted.load(Ordering::Relaxed) > 0,
        "el lote no emitió eventos"
    );
}

criterion_group! {
    name = benches;
    // Tiempos cortos para que la comparación en CI (base + PR) no tarde demasiado.
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .noise_threshold(0.03);
    targets = groserias, limpieza, kick, tiktok
}
criterion_main!(benches);
