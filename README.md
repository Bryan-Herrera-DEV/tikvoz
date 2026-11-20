# TikVoz

Lee en voz alta los comentarios de tu **TikTok LIVE** con una voz local y gratuita.
Es un solo `.exe` de ~6,5 MB con interfaz nativa: sin Electron, sin navegador y sin servicios de pago.

| | |
|---|---|
| **Latencia de voz** | ~140 ms en la primera frase y ~10 ms entre frases (voz local de Windows) |
| **Memoria** | ~75–95 MB privados (casi todo es el driver de OpenGL) |
| **CPU** | < 1 % en reposo y ~3 % con un live activo |
| **Binario** | 6,5 MB, sin instalador ni runtimes |

## Uso

1. Ejecuta `tikvoz.exe`.
2. Escribe tu `@usuario` y pulsa **Conectar**. Si todavía no estás en vivo, la app espera y se conecta sola cuando empiezas.
3. Elige la voz, la velocidad y el volumen en el panel izquierdo, y pulsa **Probar**.

También puedes crear un acceso directo con el usuario como argumento para conectar al abrir:

```
tikvoz.exe miusuario
```

### Qué hace

- **Lee** comentarios, regalos (solo al terminar la racha de combo, no "rosa x1, x2, x3…"), nuevos seguidores y compartidos. Cada uno se activa por separado y tiene su propia plantilla de lectura (`{usuario} dice: {mensaje}`).
- **Limpia** el texto antes de leerlo:
  - quita emojis;
  - reemplaza los links por "enlace";
  - acorta `jajajajajaja` a `jajaja` y `holaaaaa` a `holaaa`;
  - corta los mensajes largos sin partir palabras;
  - ignora los comandos `!algo`.
- **Anti-spam**: no repite el mismo mensaje del mismo usuario dentro de una ventana configurable (60 s por defecto).
- **Cola con tope**: si el chat va más rápido que la voz, descarta los mensajes *más viejos*. Así siempre lees lo reciente en vez de ir minutos atrasado.
- **Controles**: pausar, saltar, vaciar la cola, 🔊 para leer cualquier mensaje del feed y 🚫 para silenciar a un usuario.
- **Reconexión** automática con backoff. Si el live termina, se queda esperando a que vuelvas.
- Los ajustes se guardan solos en `%APPDATA%\TikVoz`.

### Más voces

Las voces son las de Windows: gratis, offline y sin límite. Para instalar más, ve a
**Configuración → Hora e idioma → Voz → Agregar voces** (por ejemplo, Español (España), Español (México) o Español (Colombia)).

## Compilar

Requisitos: Rust 1.84 o superior (MSVC) y Windows 10/11.

```bash
cargo build --release
```

El ejecutable queda en `target/release/tikvoz.exe`.

```bash
cargo test
```

## Arquitectura

```
┌──────────── hilo UI (egui) ────────────┐
│ app.rs   feed, ajustes, filtros        │
│   ▲ LiveEvent (mpsc)    │ texto        │
└───┼─────────────────────┼──────────────┘
    │                     ▼
┌───┴── hilo "tiktok" ───┐  ┌── hilo "voz" ───────┐
│ tokio current_thread   │  │ cola acotada        │
│ api.rs     room id +   │  │ WinRT SpeechSynth   │
│            firma       │  │ (crate `tts`)       │
│ session.rs websocket,  │  └─────────────────────┘
│   heartbeat, ack, gzip │
│ proto.rs   protobuf    │
└────────────────────────┘
```

- **UI**: `egui`/`eframe` con el backend `glow`. Es modo inmediato y solo repinta cuando llega un evento, por eso no consume CPU en reposo.
- **Red**: un runtime `tokio` de un solo hilo, `reqwest` y `tokio-tungstenite` con `rustls`, sin depender de OpenSSL.
- **Protocolo**: el webcast de TikTok es protobuf sobre WebSocket. `proto.rs` declara solo los campos que se usan con `prost`, así que el build no necesita `protoc`.
- **Firma**: TikTok exige una firma ofuscada para abrir el socket. Como todas las librerías de la comunidad (TikTokLive, TikTok-Live-Connector), se delega en el servidor público de [Euler Stream](https://www.eulerstream.com/). Funciona de forma anónima con límite compartido. Si alguna vez te devuelve "límite del servidor de firmas", crea una API key gratis y pégala en **Avanzado**.

## Limitaciones conocidas

- Es una integración **no oficial**: si TikTok cambia su protocolo, puede dejar de funcionar hasta actualizar `proto.rs` o `session.rs`.
- Las voces de Windows son robóticas comparadas con las neuronales en la nube. Esa es la contrapartida de que sean gratis, locales y sin latencia.
- Al ser un `.exe` nuevo y sin firma digital, Windows Defender o SmartScreen pueden avisar la primera vez. El ejecutable incluye metadatos de versión para reducir estos falsos positivos.
