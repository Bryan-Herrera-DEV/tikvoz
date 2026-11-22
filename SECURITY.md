# Política de seguridad

## Versiones con soporte

Solo la última versión publicada recibe correcciones de seguridad.

| Versión | Soporte |
|---|---|
| 1.0.x | Sí |
| < 1.0 | No |

## Cómo reportar una vulnerabilidad

**No abras un issue público.** Reporta la vulnerabilidad de forma privada con la función de GitHub [**Report a vulnerability**](../../security/advisories/new) (pestaña *Security* del repositorio).

Incluye, si puedes:

- La versión de TikVoz y de Windows.
- Una descripción del problema y su impacto.
- Pasos para reproducirlo o una prueba de concepto.
- Una propuesta de solución, si la tienes.

## Qué esperar

- Confirmación de recepción en un plazo de **7 días**.
- Una evaluación inicial y un plan en un plazo de **14 días**.
- Te mantendremos al tanto del avance y te daremos crédito en la nota de la versión que lo corrija, salvo que prefieras quedar en el anonimato.

Pedimos divulgación coordinada: no publiques los detalles hasta que haya una versión corregida.

## Alcance

Es especialmente relevante cualquier problema que permita:

- Ejecutar código o leer archivos del equipo a partir de mensajes del chat o de respuestas de TikTok, Kick o de los servidores que usa la app.
- Exponer la API key de Euler Stream u otros datos guardados en los ajustes.
- Que un ejecutable publicado en *Releases* no corresponda al código del repositorio.

Quedan fuera de alcance: evadir el filtro de groserías (repórtalo como issue normal con la plantilla **Filtro de groserías**), y los cambios de protocolo de TikTok o Kick que dejen de funcionar.

## Verificar los ejecutables

Cada versión publica sumas SHA-256 y una atestación de procedencia firmada por GitHub Actions. Consulta la sección *Verificar la descarga* del [README](README.md#instalación).
