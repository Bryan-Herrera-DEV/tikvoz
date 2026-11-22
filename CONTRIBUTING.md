# Cómo contribuir a TikVoz

Gracias por tu interés. Cualquier aporte suma: reportar un error, proponer una mejora, ampliar la lista de groserías, mejorar la documentación o enviar código.

Al participar aceptas el [Código de conducta](CODE_OF_CONDUCT.md). Para vulnerabilidades de seguridad no abras un issue público: sigue la [Política de seguridad](SECURITY.md).

## Índice

- [Formas de contribuir](#formas-de-contribuir)
- [Antes de empezar](#antes-de-empezar)
- [Entorno de desarrollo](#entorno-de-desarrollo)
- [Flujo de trabajo](#flujo-de-trabajo)
- [Estilo de código](#estilo-de-código)
- [Mensajes de commit](#mensajes-de-commit)
- [Pull requests](#pull-requests)
- [Guías por área](#guías-por-área)
- [Publicar una versión](#publicar-una-versión)

## Formas de contribuir

| Quiero… | Cómo |
|---|---|
| Reportar un error | Abre un issue con la plantilla **Reporte de error**. |
| Proponer una función | Abre un issue con la plantilla **Solicitud de función** y espera a que se discuta antes de programarla. |
| Pedir una plataforma nueva (YouTube, Twitch…) | Usa la plantilla **Nueva plataforma**. |
| Reportar un falso positivo o una grosería que no se detecta | Usa la plantilla **Filtro de groserías**. |
| Corregir documentación o un error pequeño | Envía directamente un pull request. |

Los issues etiquetados `good first issue` son un buen punto de partida.

## Antes de empezar

- Busca en los [issues](../../issues) por si ya existe algo parecido.
- Para cambios grandes (una plataforma nueva, un motor de voz, cambios de arquitectura), abre primero un issue y espera a que se discuta. Así nadie pierde tiempo en algo que no encaja.
- TikVoz prioriza **bajo consumo y baja latencia**. Una dependencia o función que aumente mucho la RAM, el tamaño del `.exe` o la latencia necesita una justificación clara.

## Entorno de desarrollo

Requisitos:

- Windows 10 u 11 (es la plataforma soportada; el código compila en Linux y macOS, pero no se prueba ahí).
- [Rust](https://rustup.rs) **1.84 o superior** con la toolchain MSVC. 1.84 es la versión mínima soportada (MSRV) y CI lo verifica.
- Visual Studio Build Tools con el componente "Desarrollo para el escritorio con C++" (lo pide `rustup`).

```bash
git clone https://github.com/Bryan-Herrera-DEV/tikvoz.git
cd tikvoz
cargo run                      # compila y abre la app en modo debug
cargo run -- --demo            # abre el live simulado (no necesita estar en vivo)
cargo run -- --config ./tmp    # usa ajustes aislados en ./tmp sin tocar los tuyos
```

Antes de enviar cambios, ejecuta lo mismo que ejecuta CI:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Para generar el paquete de una versión igual que GitHub Actions:

```bash
pwsh scripts/package.ps1
```

## Flujo de trabajo

1. Haz un fork y crea una rama desde `main` con un nombre descriptivo: `fix/kick-reconexion`, `feat/youtube`, `docs/faq`.
2. Haz commits pequeños y atómicos (ver [Mensajes de commit](#mensajes-de-commit)).
3. Agrega o actualiza tests para el comportamiento que cambias.
4. Actualiza la documentación (README, CHANGELOG en la sección `[Sin publicar]`) si el cambio lo afecta.
5. Abre el pull request contra `main` y completa la plantilla.

## Estilo de código

- **Formato:** el de `rustfmt` por defecto (`cargo fmt`). CI lo exige.
- **Lints:** `cargo clippy` sin advertencias. Si un lint no aplica, desactívalo de forma local con `#[allow(...)]` y un comentario que explique por qué.
- **Idioma:** comentarios, mensajes de la interfaz y documentación en español. Los nombres en el código siguen las convenciones de Rust.
- **Comentarios:** explica el *porqué*, no el *qué*. Las decisiones no obvias (protocolo, rendimiento, falsos positivos) deben quedar documentadas junto al código.
- **Errores:** sin `unwrap()` en rutas que pueden fallar en producción (red, archivos, protocolo). Los mensajes de error que ve el usuario van en español claro.
- **Sin `unsafe`** salvo para FFI con Windows, con un comentario `// SAFETY:` que lo justifique.
- **Dependencias:** compatibles con Rust 1.84 y con licencia permisiva (MIT, Apache-2.0, BSD o similar).

## Mensajes de commit

Usamos [Conventional Commits](https://www.conventionalcommits.org/es/v1.0.0/), con la descripción en español, en minúscula y en modo imperativo o descriptivo breve:

```
<tipo>(<ámbito opcional>): <descripción corta>
```

| Tipo | Para |
|---|---|
| `feat` | una función nueva |
| `fix` | corrección de un error |
| `docs` | solo documentación |
| `style` | formato, sin cambios de lógica |
| `refactor` | reestructura sin cambiar el comportamiento |
| `perf` | mejora de rendimiento |
| `test` | tests nuevos o corregidos |
| `build` | compilación, dependencias, empaquetado |
| `ci` | workflows de GitHub Actions |
| `chore` | mantenimiento que no encaja en los anteriores |

Ámbitos habituales: `tiktok`, `kick`, `voz`, `filtro`, `groserías`, `ui`, `monitor`, `config`.

Ejemplos:

```
feat(kick): lee los mensajes destacados
fix(groserías): evita marcar "computadora" como falso positivo
docs: aclara cómo instalar voces en español
```

## Pull requests

- Un pull request resuelve **una sola cosa**. Si arreglas dos errores no relacionados, envía dos.
- CI debe pasar: formato, clippy, tests y compilación con la MSRV.
- Explica qué cambia, por qué y cómo lo probaste. Si cambia la interfaz, agrega una captura (puedes usar `--demo --config ./tmp` para no mostrar datos reales).
- Los mantenedores pueden pedir cambios. Al aprobarse, se integra con *squash* o *rebase* para mantener un historial limpio.

## Guías por área

### Filtro de groserías (`src/profanity.rs`)

- Cada palabra nueva en la lista por defecto debe ir en su `[categoría]` y con un caso en los tests de detección.
- **Cada cambio debe incluir también casos de palabras normales que no deben marcarse.** Los falsos positivos (el "problema de Scunthorpe") son tan graves como lo que no se detecta.
- Prefiere la palabra exacta antes que un prefijo (`palabra*`), y un prefijo antes que una coincidencia en cualquier parte (`*palabra*`).
- No cambies el formato de la lista sin agregar una migración en `config.rs`: los usuarios tienen listas guardadas.

### Plataformas (`src/tiktok/`, `src/kick/`)

Una plataforma nueva vive en su propio módulo y:

1. Corre en su propio hilo con un runtime `tokio` de un solo hilo.
2. Traduce su protocolo a `LiveEvent` (`src/live.rs`). La interfaz, los filtros y la voz no deben saber de qué plataforma viene un mensaje.
3. Se reconecta sola con `Backoff` y reporta su estado con `LiveEvent::Status`.
4. Separa el parseo del protocolo en funciones puras con tests basados en mensajes reales capturados (anonimiza usuarios y contenido).
5. Documenta en el README el flujo de conexión con un diagrama Mermaid.

Las integraciones son **solo de lectura**: TikVoz no envía mensajes ni realiza acciones en nombre del streamer.

### Voz (`src/speech.rs`)

Cualquier cambio debe mantener la latencia (primera frase en menos de 200 ms con las voces de Windows) y la cola con tope. Si propones un motor de voz nuevo, mide la RAM y la latencia antes y después e inclúyelas en el pull request.

### Interfaz (`src/app.rs`)

- La interfaz solo se redibuja ante eventos: no agregues animaciones ni repintados continuos.
- Usa los colores del tema (`ui.visuals()`) para que todo se vea bien en modo claro y oscuro.
- Todo ajuste nuevo va en `config.rs` con `#[serde(default)]` y un valor por defecto razonable, para que los archivos de ajustes viejos sigan funcionando.

## Publicar una versión

Solo para mantenedores:

1. Mueve las entradas de `[Sin publicar]` a una nueva sección `## [x.y.z] - AAAA-MM-DD` en el [CHANGELOG](CHANGELOG.md).
2. Actualiza `version` en `Cargo.toml` y ejecuta `cargo check` para actualizar `Cargo.lock`.
3. Haz commit (`chore(release): vx.y.z`), crea el tag anotado y súbelo:

   ```bash
   git tag -a vx.y.z -m "TikVoz x.y.z"
   git push origin main vx.y.z
   ```

4. El workflow `Release` verifica, compila, empaqueta y publica. Revisa la release en GitHub cuando termine.

Usamos [versionado semántico](https://semver.org/lang/es/): `MAJOR` para cambios incompatibles (por ejemplo, en el formato de los ajustes), `MINOR` para funciones nuevas y `PATCH` para correcciones.

## Licencia

Al contribuir aceptas que tu aporte se publique bajo la [licencia MIT](LICENSE) del proyecto.
