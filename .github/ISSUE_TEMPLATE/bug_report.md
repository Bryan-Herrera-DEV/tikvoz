---
name: Reporte de error
about: Algo no funciona como debería
title: "[Error] "
labels: bug
assignees: ""
---

<!--
Gracias por reportarlo. Antes de enviar:
- Busca en los issues por si ya existe.
- Si es una vulnerabilidad de seguridad, NO la reportes aquí: usa la pestaña Security.
- No incluyas tu API key ni datos privados de tu canal.
-->

## Descripción

Explica de forma clara y breve qué pasa.

## Pasos para reproducirlo

1. Abrir TikVoz con …
2. Conectar a …
3. …
4. Ver el error

## Comportamiento esperado

Qué esperabas que pasara.

## Comportamiento actual

Qué pasó en realidad. Incluye el texto exacto del estado o del error que muestra la app.

## Capturas o registros

Si aplica, agrega capturas. Para obtener un registro detallado, ejecuta desde una terminal:

```powershell
$env:RUST_LOG="tikvoz=debug"; .\tikvoz.exe
```

<details>
<summary>Registro</summary>

```text
pega aquí el registro
```

</details>

## Entorno

- Versión de TikVoz: <!-- p. ej. 1.0.0 (aparece en el título de la ventana) -->
- Windows: <!-- p. ej. Windows 11 24H2 -->
- Plataforma afectada: <!-- TikTok / Kick / Demo / ninguna -->
- Voz usada: <!-- p. ej. Microsoft Sabina -->
- ¿Usas modo portable (`--config`)? <!-- sí / no -->

## Información adicional

Cualquier otro detalle que ayude: si pasa siempre o a veces, desde qué versión, etc.
