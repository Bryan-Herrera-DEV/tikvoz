//! Limpieza de texto antes de mandarlo a la voz.
//!
//! Todo aquí es puro (sin IO) para poder testearlo fácil. El objetivo es que el
//! TTS no lea basura: links, cadenas de emojis, "jajajajajajaja" de 80 letras, etc.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

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
    /// `true`: las palabras bloqueadas se leen como "piip"; `false`: se omite el comentario.
    pub censor_instead_of_skip: bool,
    /// Si no está vacío, solo se leen comentarios que empiecen con esto (y se quita).
    pub required_prefix: String,
    /// Diccionario de pronunciación, una regla por línea: `xd = equis de`.
    pub dictionary: String,
}

/// Abreviaturas típicas del chat en español para que la voz las diga bien.
pub const DEFAULT_DICTIONARY: &str = "xd = equis de
q = que
k = que
pq = porque
tqm = te quiero mucho
tkm = te quiero mucho
bn = bien
xq = porque
npi = ni idea
dnd = dónde
grax = gracias
wtf = qué
ntp = no te preocupes";

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            max_chars: 200,
            min_chars: 1,
            strip_links: true,
            skip_commands: true,
            blocked_words: String::new(),
            blocked_users: String::new(),
            censor_instead_of_skip: false,
            required_prefix: String::new(),
            dictionary: DEFAULT_DICTIONARY.into(),
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

    let mut trimmed = raw.trim();
    let prefix = cfg.required_prefix.trim();
    if !prefix.is_empty() {
        let head = trimmed.get(..prefix.len())?;
        if !head.eq_ignore_ascii_case(prefix) {
            return None;
        }
        trimmed = trimmed[prefix.len()..].trim_start();
    } else if cfg.skip_commands && trimmed.starts_with('!') {
        return None;
    }
    let censored;
    if contains_blocked_word(trimmed, &cfg.blocked_words) {
        if !cfg.censor_instead_of_skip {
            return None;
        }
        censored = censor(trimmed, &cfg.blocked_words);
        trimmed = &censored;
    }

    let mut text = if cfg.strip_links {
        replace_links(trimmed)
    } else {
        trimmed.to_string()
    };
    text = strip_unspeakable(&text);
    text = collapse_runs(&text);
    text = apply_dictionary(&text, &cfg.dictionary);
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

/// Aplica `f` a cada palabra (secuencia alfanumérica) y conserva el resto tal cual.
fn map_words(text: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            match f(word) {
                Some(r) => out.push_str(&r),
                None => out.push_str(word),
            }
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Reemplaza palabras completas según el diccionario (`clave = valor` por línea).
pub fn apply_dictionary(text: &str, dictionary: &str) -> String {
    let rules: Vec<(String, &str)> = dictionary
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let k = k.trim().to_lowercase();
            (!k.is_empty()).then(|| (k, v.trim()))
        })
        .collect();
    if rules.is_empty() {
        return text.to_string();
    }
    map_words(text, |w| {
        let lw = w.to_lowercase();
        rules.iter().find(|(k, _)| *k == lw).map(|(_, v)| v.to_string())
    })
}

fn censor(text: &str, blocked: &str) -> String {
    let words: Vec<String> = split_list(blocked).collect();
    map_words(text, |w| {
        let lw = w.to_lowercase();
        words.iter().any(|b| lw.contains(b.as_str())).then(|| "piip".to_string())
    })
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
            if c.is_alphanumeric() || c.is_whitespace() || ".,;:¿?¡!'\"-()%$&+=/".contains(c) {
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

/// Recuerda qué dijo cada usuario hace poco para no leer el mismo mensaje
/// repetido (en lives de ventas la gente pega la misma pregunta 5 veces).
pub struct RepeatGuard {
    window: Duration,
    seen: VecDeque<(Instant, String)>,
}

impl RepeatGuard {
    pub fn new(window: Duration) -> Self {
        Self { window, seen: VecDeque::new() }
    }

    pub fn set_window(&mut self, window: Duration) {
        self.window = window;
    }

    /// `true` si es nuevo (y lo registra); `false` si es una repetición reciente.
    pub fn check(&mut self, user: &str, text: &str, now: Instant) -> bool {
        while self
            .seen
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > self.window)
        {
            self.seen.pop_front();
        }
        let key = format!("{}\u{1f}{}", user.to_lowercase(), text.to_lowercase());
        if self.seen.iter().any(|(_, k)| *k == key) {
            return false;
        }
        self.seen.push_back((now, key));
        true
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
    fn repeat_guard_ignora_repeticiones_dentro_de_la_ventana() {
        let mut g = RepeatGuard::new(Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(g.check("ana", "Hola", t0));
        assert!(!g.check("ANA", "hola", t0 + Duration::from_secs(10)));
        assert!(g.check("luis", "hola", t0 + Duration::from_secs(10)));
        assert!(g.check("ana", "hola", t0 + Duration::from_secs(61)));
    }

    #[test]
    fn diccionario_reemplaza_palabras_completas() {
        let d = "xd = equis de
q = que";
        assert_eq!(apply_dictionary("XD q risa, qué", d), "equis de que risa, qué");
        assert_eq!(apply_dictionary("xdxd", d), "xdxd");
    }

    #[test]
    fn censura_en_vez_de_omitir() {
        let c = FilterConfig {
            blocked_words: "tonto".into(),
            censor_instead_of_skip: true,
            ..cfg()
        };
        assert_eq!(clean_comment("eres un TONTO jaja", &[], &c).as_deref(), Some("eres un piip jaja"));
    }

    #[test]
    fn prefijo_obligatorio() {
        let c = FilterConfig { required_prefix: "!v".into(), ..cfg() };
        assert_eq!(clean_comment("hola", &[], &c), None);
        assert_eq!(clean_comment("!V hola a todos", &[], &c).as_deref(), Some("hola a todos"));
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
