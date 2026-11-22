# Changelog

Todos los cambios importantes de TikVoz se documentan aquí.
El formato sigue [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/) y el proyecto usa [versionado semántico](https://semver.org/lang/es/).

## [Sin publicar]

## [1.0.0] - 2026-11-22

Primera versión estable. 🎉

### Plataformas
- **TikTok LIVE**: comentarios, regalos (combos leídos una sola vez), seguidores, compartidos y bienvenidas. Reconexión automática y espera hasta que empiece el live.
- **Kick** *(experimental)*: comentarios, suscripciones, suscripciones regaladas y Kicks mediante el nuevo chat de Kick (Centrifugo). El protocolo cambió en octubre de 2026 y esta integración aún no se probó contra un canal en vivo: si falla, abre un issue.
- Ambas plataformas a la vez, en una sola cola de voz.
- **Modo demo** (`--demo`) para probar sin estar en vivo.

### Voz
- Voces de Windows locales, gratis y offline. Primera frase en ~140 ms.
- Cola con tope: si el chat va más rápido que la voz, se descartan los mensajes más viejos.
- Prioridad para regalos, subs, seguidores, VIP y moderadores.
- Voz distinta por usuario.
- Plantillas de lectura por tipo de evento.

### Moderación
- Detector de groserías resistente a ofuscación:
  - separadores (`pi to`, `a.buso`);
  - leetspeak (`p1to`, `4buso`);
  - repeticiones;
  - letras "fancy", versalitas, banderas y recuadros;
  - homoglifos (UTS #39);
  - texto invertido o de cabeza;
  - caracteres invisibles;
  - escritura fonética y abreviaturas.
- Categorías activables (insultos, vulgar, sexual, odio, violencia, abreviaturas, inglés), excepciones, comodines estilo Discord y frases.
- Censura ("piip") u omisión, auto-silencio tras N faltas y nombres ofensivos leídos como "alguien".
- Anti-spam, enfriamiento por usuario, solo seguidores (TikTok), modo prefijo, links → "enlace", diccionario de pronunciación.

### Aplicación
- Interfaz nativa (egui): tema claro, oscuro o del sistema.
- Monitor de CPU, RAM, GPU y VRAM del propio programa.
- Modo portable (`--config <carpeta>`) y acceso directo (`tikvoz.exe usuario --kick canal`).
- Un solo `.exe` de ~7 MB, ~100 MB de RAM y < 1 % de CPU.

[Sin publicar]: ../../compare/v1.0.0...HEAD
[1.0.0]: ../../releases/tag/v1.0.0
