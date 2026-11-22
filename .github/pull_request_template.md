## Qué cambia

<!-- Describe el cambio en una o dos frases. -->

Cierra #<!-- número del issue, si existe -->

## Tipo de cambio

- [ ] Corrección de error (`fix`)
- [ ] Función nueva (`feat`)
- [ ] Cambio incompatible (rompe ajustes guardados o el comportamiento actual)
- [ ] Documentación (`docs`)
- [ ] Refactor, rendimiento o mantenimiento (`refactor`, `perf`, `chore`)

## Cómo se probó

<!--
Pasos para verificarlo. Si cambia la interfaz, agrega una captura.
Puedes usar `cargo run -- --demo --config ./tmp` para no mostrar datos reales.
-->

## Rendimiento

<!-- Solo si aplica: RAM, tamaño del .exe o latencia de la voz antes y después. -->

## Lista de verificación

- [ ] `cargo fmt --check` pasa.
- [ ] `cargo clippy --all-targets --locked -- -D warnings` no muestra advertencias.
- [ ] `cargo test --locked` pasa, y agregué tests para lo que cambié.
- [ ] Si toqué el filtro de groserías, agregué casos que **sí** deben detectarse y casos que **no**.
- [ ] Si agregué un ajuste, tiene valor por defecto y los ajustes viejos siguen cargando.
- [ ] Actualicé el README y el CHANGELOG (sección `[Sin publicar]`) si correspondía.
- [ ] Los commits siguen [Conventional Commits](https://www.conventionalcommits.org/es/v1.0.0/).
- [ ] Leí y acepto la [guía de contribución](https://github.com/Bryan-Herrera-DEV/tikvoz/blob/main/CONTRIBUTING.md).
