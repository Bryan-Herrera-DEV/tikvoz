//! Limpieza de texto antes de mandarlo a la voz.
//!
//! Todo aquí es puro (sin IO) para poder testearlo fácil. El objetivo es que el
//! TTS no lea basura: links, cadenas de emojis, "jajajajajajaja" de 80 letras, etc.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FilterConfig {
    /// Longitud máxima (en caracteres) que se lee; el resto se corta.
    pub max_chars: usize,
    /// Comentarios más cortos que esto (ya limpios) se ignoran.
    pub min_chars: usize,
    /// Reemplaza URLs por la palabra "enlace".
    pub strip_links: bool,
    /// Ignora comentarios que empiezan por `!` (comandos de bots).
    pub skip_commands: bool,
    /// Palabras prohibidas separadas por coma: si aparece alguna, no se lee.
    pub blocked_words: String,
    /// Usuarios (@usuario o nick) separados por coma que nunca se leen.
    pub blocked_users: String,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            max_chars: 200,
            min_chars: 1,
            strip_links: true,
            skip_commands: true,
            blocked_words: String::new(),
            blocked_users: String::new(),
        }
    }
}

/// Repeticiones máximas permitidas de un mismo carácter seguido ("holaaaaa" -> "holaaa").
const MAX_CHAR_RUN: usize = 3;
/// Repeticiones máximas de una misma sílaba de 2 letras ("jajajaja..." -> "jajaja").
const MAX_PAIR_RUN: usize = 3;

/// Devuelve `None` si el comentario no debe leerse, o el texto limpio si sí.
pub fn clean_comment(raw: &str, user_ids: &[&str], cfg: &FilterConfig) -> Option<String> {
    if is_user_blocked(user_ids, &cfg.blocked_users) {
        return None;
    }

    let trimmed = raw.trim();
    if cfg.skip_commands && trimmed.starts_with('!') {
        return None;
    }
    if contains_blocked_word(trimmed, &cfg.blocked_words) {
        return None;
    }

    let mut text = if cfg.strip_links {
        replace_links(trimmed)
    } else {
        trimmed.to_string()
    };
    text = strip_unspeakable(&text);
    text = collapse_runs(&text);
    text = collapse_whitespace(&text);

    if text.chars().filter(|c| c.is_alphanumeric()).count() < cfg.min_chars.max(1) {
        return None;
    }

    Some(truncate_chars(&text, cfg.max_chars))
}

/// Limpia un nombre para leerlo en voz alta (sin emojis ni símbolos raros).
pub fn clean_name(raw: &str) -> String {
    let cleaned = collapse_whitespace(&collapse_runs(&strip_unspeakable(raw)));
    if cleaned.chars().any(|c| c.is_alphanumeric()) {
        cleaned
    } else {
        String::new()
    }
}

fn split_list(list: &str) -> impl Iterator<Item = String> + '_ {
    list.split(',')
        .map(|s| s.trim().trim_start_matches('@').to_lowercase())
        .filter(|s| !s.is_empty())
}

fn is_user_blocked(user_ids: &[&str], blocked: &str) -> bool {
    split_list(blocked).any(|b| {
        user_ids
            .iter()
            .any(|u| u.trim_start_matches('@').to_lowercase() == b)
    })
}

fn contains_blocked_word(text: &str, blocked: &str) -> bool {
    let lower = text.to_lowercase();
    split_list(blocked).any(|w| lower.contains(&w))
}

fn replace_links(text: &str) -> String {
    text.split_whitespace()
        .map(|w| {
            let lw = w.to_lowercase();
            let looks_like_link = lw.starts_with("http://")
                || lw.starts_with("https://")
                || lw.starts_with("www.")
                || [".com", ".net", ".org", ".io", ".ly", ".gg", ".tv"]
                    .iter()
                    .any(|tld| lw.trim_end_matches('/').ends_with(tld) && lw.len() > tld.len() + 1);
            if looks_like_link {
                "enlace"
            } else {
                w
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quita emojis, pictogramas y símbolos que el TTS lee mal o deletrea.
/// Mantiene letras (cualquier alfabeto), números, espacios y puntuación básica.
fn strip_unspeakable(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else if ".,;:¿?¡!'\"-()%$&+=/".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect()
}

/// Recorta repeticiones: "holaaaaaa" -> "holaaa", "jajajajajaja" -> "jajaja", "!!!!!!" -> "!!!".
fn collapse_runs(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();

    // 1) Carácter repetido.
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut run = 0;
    for (i, &c) in chars.iter().enumerate() {
        let same = i > 0 && chars[i - 1].to_lowercase().eq(c.to_lowercase());
        run = if same { run + 1 } else { 1 };
        if run <= MAX_CHAR_RUN {
            out.push(c);
        }
    }

    // 2) Par repetido (ja, je, ha, xd...).
    let src = out;
    let mut out: Vec<char> = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if i + 1 < src.len() && src[i].is_alphabetic() && src[i + 1].is_alphabetic() {
            let pair = [src[i].to_ascii_lowercase(), src[i + 1].to_ascii_lowercase()];
            let mut reps = 1;
            let mut j = i + 2;
            while j + 1 < src.len()
                && src[j].to_ascii_lowercase() == pair[0]
                && src[j + 1].to_ascii_lowercase() == pair[1]
            {
                reps += 1;
                j += 2;
            }
            if reps > 1 {
                let keep = reps.min(MAX_PAIR_RUN);
                out.extend_from_slice(&src[i..i + keep * 2]);
                i = j;
                continue;
            }
        }
        out.push(src[i]);
        i += 1;
    }

    out.into_iter().collect()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if max == 0 || text.chars().count() <= max {
        return text.to_string();
    }
    // Cortamos en el último espacio para no partir palabras a la mitad.
    let cut: String = text.chars().take(max).collect();
    if text.chars().nth(max).is_some_and(char::is_whitespace) {
        return cut; // el corte cae justo en un límite de palabra
    }
    match cut.rfind(' ') {
        Some(pos) if pos > max / 2 => cut[..pos].to_string(),
        _ => cut,
    }
}

/// Sustituye `{usuario}` y `{mensaje}` en la plantilla de lectura.
pub fn render_template(template: &str, user: &str, message: &str) -> String {
    let out = template.replace("{usuario}", user).replace("{mensaje}", message);
    collapse_whitespace(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FilterConfig {
        FilterConfig::default()
    }

    #[test]
    fn deja_pasar_texto_normal() {
        assert_eq!(
            clean_comment("Hola, ¿cómo estás?", &[], &cfg()).as_deref(),
            Some("Hola, ¿cómo estás?")
        );
    }

    #[test]
    fn recorta_risas_y_letras_repetidas() {
        assert_eq!(
            clean_comment("jajajajajajajaja holaaaaaaa", &[], &cfg()).as_deref(),
            Some("jajaja holaaa")
        );
    }

    #[test]
    fn quita_emojis_y_descarta_si_solo_hay_emojis() {
        assert_eq!(clean_comment("🔥🔥🔥😂", &[], &cfg()), None);
        assert_eq!(
            clean_comment("buenísimo 🔥🔥", &[], &cfg()).as_deref(),
            Some("buenísimo")
        );
    }

    #[test]
    fn reemplaza_links() {
        assert_eq!(
            clean_comment("mira https://spam.example/x ya", &[], &cfg()).as_deref(),
            Some("mira enlace ya")
        );
        assert_eq!(
            clean_comment("entra a pagina.com", &[], &cfg()).as_deref(),
            Some("entra a enlace")
        );
    }

    #[test]
    fn ignora_comandos() {
        assert_eq!(clean_comment("!dado", &[], &cfg()), None);
    }

    #[test]
    fn bloquea_palabras_y_usuarios() {
        let c = FilterConfig {
            blocked_words: "spam, feo".into(),
            blocked_users: "@troll123".into(),
            ..cfg()
        };
        assert_eq!(clean_comment("esto es SPAM", &[], &c), None);
        assert_eq!(clean_comment("hola", &["troll123", "Troll"], &c), None);
        assert!(clean_comment("hola", &["amigo"], &c).is_some());
    }

    #[test]
    fn trunca_en_limite_de_palabra() {
        let c = FilterConfig { max_chars: 12, ..cfg() };
        assert_eq!(
            clean_comment("uno dos tres cuatro cinco", &[], &c).as_deref(),
            Some("uno dos tres")
        );
    }

    #[test]
    fn limpia_nombres() {
        assert_eq!(clean_name("✨Ana✨"), "Ana");
        assert_eq!(clean_name("🔥🔥"), "");
    }

    #[test]
    fn plantilla() {
        assert_eq!(
            render_template("{usuario} dice {mensaje}", "Ana", "hola"),
            "Ana dice hola"
        );
        assert_eq!(render_template("{usuario} dice {mensaje}", "", "hola"), "dice hola");
    }
}
