//! Detector de groserías resistente a ofuscación.
//!
//! La gente evade los filtros simples con trucos como `pi to`, `p1to`, `4buso`,
//! `a.buso`, `p*ta`, `piiiito`, `𝓹𝓲𝓽𝓸`, letras cirílicas (`рito`), caracteres
//! invisibles o escritura fonética (`berga`, `kabron`). Este módulo normaliza el
//! texto en varias capas y compara contra la lista ya normalizada:
//!
//! 1. **Unicode**: NFKC (letras "fancy", ancho completo, círculos → ASCII), se
//!    quitan acentos/diacríticos y caracteres invisibles (zero-width, soft hyphen).
//! 2. **Homoglifos**: cirílico/griego que parecen latinos (`а е о р с у х і`…).
//! 3. **Leetspeak**: `0→o 1→i 3→e 4→a @→a 5→s $→s 7→t 8→b 9→g !→i`…
//! 4. **Fonética del español**: `v→b`, `z→s`, `ce/ci→se/si`, `c/q/k→k`,
//!    `qu(e|i)→k`, `ge/gi→je/ji`, `ch→ç`, `y→i`, `h` muda.
//! 5. **Repeticiones**: `piiiito → pito`.
//! 6. **Separadores**: `.`, `-`, `_`, espacios… Una groserías puede abarcar varios
//!    fragmentos (`pi to`, `p.u.t.o`) pero la coincidencia debe empezar y terminar
//!    justo en los bordes de esos fragmentos. Así `pi to` se detecta pero
//!    `papi tomo` o `computo` no.
//!
//! Sintaxis de la lista: una palabra por línea o separadas por coma. `palabra*`
//! detecta todo lo que empiece así (`pendej*` → pendejo, pendejada…). Sin `*` se
//! detecta la palabra exacta y su plural (`-s`, `-es`). En el texto, un `*`
//! funciona como comodín de una letra (`p*ta`).

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Lista por defecto (español + algo de inglés). Editable desde la app.
pub const DEFAULT_BAD_WORDS: &str = "\
puta, puto, putita, putito, putamadre*, putamare*, hdp, hijueput*, hijodeput*, hijoeput*,
pendej*, verga*, pito, polla, pinga, mierda*, cabron*, chinga*, culer*, culo,
marica*, maricon*, joto, zorra, perra, idiota, imbecil, estupid*, gilipollas, malparid*,
gonorrea, careverga, coño, conchatumadre*, conchetumadre*, conchesumadre*, mamaguev*, mamahuev*,
cojud*, follar, violar*, violacion, violador*, abuso, pedofil*, nazi, matate, suicidate,
fuck*, shit*, bitch*, nigger*, nigga*, faggot*, cunt*, whore*, rape, rapist*";

/// Un fragmento detectado, con su rango de bytes en el texto original.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub start: usize,
    pub end: usize,
    /// La entrada de la lista que coincidió (tal como la escribió el usuario).
    pub rule: String,
}

#[derive(Debug, Clone)]
struct Rule {
    /// Letras normalizadas con cuántas veces se repiten seguidas ("perra" → r×2).
    pattern: Vec<(char, u8)>,
    prefix: bool,
    label: String,
}

/// Lista compilada. Construirla cuesta algo; se reconstruye solo si la lista cambia.
#[derive(Debug, Clone, Default)]
pub struct Profanity {
    rules: Vec<Rule>,
}

/// Letra normalizada con su origen en el texto.
#[derive(Debug, Clone, Copy)]
struct Unit {
    ch: char,
    /// Repeticiones seguidas de esta letra ("piiiito" → i×4).
    count: u8,
    start: usize,
    end: usize,
    token: usize,
}

const WILDCARD: char = '*';
/// `ch` se representa con un solo símbolo para no confundirse con `c`/`k`.
const CH: char = 'ç';

impl Profanity {
    pub fn new(list: &str) -> Self {
        let rules = list
            .split([',', '\n', ';'])
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .filter_map(|w| {
                let prefix = w.ends_with('*');
                let base = w.trim_end_matches('*');
                let pattern: Vec<(char, u8)> = normalize(base)
                    .into_iter()
                    .filter(|u| u.ch != WILDCARD)
                    .map(|u| (u.ch, u.count))
                    .collect();
                // Patrones de menos de 3 letras dan demasiados falsos positivos.
                (pattern.len() >= 3).then(|| Rule { pattern, prefix, label: w.to_string() })
            })
            .collect();
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn contains(&self, text: &str) -> bool {
        !self.find(text).is_empty()
    }

    /// Reemplaza cada coincidencia por `replacement`.
    pub fn censor(&self, text: &str, replacement: &str) -> String {
        let hits = self.find(text);
        if hits.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for h in hits {
            out.push_str(&text[last..h.start]);
            out.push_str(replacement);
            last = h.end;
        }
        out.push_str(&text[last..]);
        out
    }

    pub fn find(&self, text: &str) -> Vec<Hit> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        let units = normalize(text);
        let mut hits = Vec::new();
        let mut i = 0;
        while i < units.len() {
            let token_start = i == 0 || units[i - 1].token != units[i].token;
            if token_start {
                if let Some((end, rule)) = self.match_at(&units, i) {
                    hits.push(Hit {
                        start: units[i].start,
                        end: units[end - 1].end,
                        rule: rule.label.clone(),
                    });
                    i = end;
                    continue;
                }
            }
            i += 1;
        }
        hits
    }

    /// Intenta cada regla empezando en `i`; devuelve el índice final (exclusivo).
    fn match_at<'a>(&'a self, u: &[Unit], i: usize) -> Option<(usize, &'a Rule)> {
        // Preferimos la coincidencia más larga (p. ej. "putamadre" sobre "puta").
        let mut best: Option<(usize, &Rule)> = None;
        for rule in &self.rules {
            let Some(end) = match_rule(u, i, rule) else { continue };
            if best.is_none_or(|(e, _)| end > e) {
                best = Some((end, rule));
            }
        }
        best
    }
}

fn match_rule(u: &[Unit], i: usize, rule: &Rule) -> Option<usize> {
    let p = &rule.pattern;
    if i + p.len() > u.len() {
        return None;
    }
    let mut wildcards = 0;
    for (k, &(pc, pn)) in p.iter().enumerate() {
        let x = u[i + k];
        if x.ch == WILDCARD {
            wildcards += 1;
        } else if x.ch != pc || x.count < pn {
            // "piiiito" (i×4) cumple "pito" (i×1), pero "pera" (r×1) no cumple "perra" (r×2).
            return None;
        }
    }
    // Un comodín cada 4 letras como mucho: "****" no debe coincidir con todo.
    if wildcards > (p.len() / 4).max(1) || wildcards == p.len() {
        return None;
    }
    let mut end = i + p.len();
    let at_boundary = |e: usize| e == u.len() || u[e].token != u[e - 1].token;

    if rule.prefix {
        // Consumimos el resto del fragmento: "pendej*" cubre "pendejada" completo.
        while !at_boundary(end) {
            end += 1;
        }
    } else if !at_boundary(end) {
        // Plural: -s / -es.
        let tail: String = u[end..].iter().take(2).map(|x| x.ch).collect();
        if tail.starts_with('s') && at_boundary(end + 1) {
            end += 1;
        } else if tail == "es" && end + 2 <= u.len() && at_boundary(end + 2) {
            end += 2;
        } else {
            return None;
        }
    }

    if !plausible_split(u, i, end) {
        return None;
    }
    Some(end)
}

/// Si la coincidencia abarca varios fragmentos ("pi to"), solo la aceptamos cuando
/// parece ofuscación: todos los fragmentos salvo el más largo son cortos (≤ 3).
/// Dos palabras normales juntas ("mama gueva") rara vez cumplen eso.
fn plausible_split(u: &[Unit], start: usize, end: usize) -> bool {
    let first = u[start].token;
    let last = u[end - 1].token;
    if first == last {
        return true;
    }
    let mut lens: Vec<usize> = Vec::new();
    let mut cur = first;
    let mut len = 0;
    for x in &u[start..end] {
        if x.token != cur {
            lens.push(len);
            cur = x.token;
            len = 0;
        }
        len += 1;
    }
    lens.push(len);
    let longest = lens.iter().copied().max().unwrap_or(0);
    let mut skipped_longest = false;
    lens.into_iter().all(|l| {
        if l == longest && !skipped_longest {
            skipped_longest = true;
            true
        } else {
            l <= 3
        }
    })
}

/// Caracteres que no ocupan espacio visual: se ignoran por completo.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{00AD}' | '\u{034F}' | '\u{180E}')
        || is_combining_mark(c)
        || ('\u{FE00}'..='\u{FE0F}').contains(&c)
}

fn homoglyph(c: char) -> Option<char> {
    Some(match c {
        'а' | 'α' => 'a',
        'в' | 'β' => 'b',
        'с' | 'ς' => 'c',
        'е' | 'ε' | 'ё' => 'e',
        'н' | 'η' => 'h',
        'і' | 'ї' | 'ι' | 'ı' => 'i',
        'ј' => 'j',
        'к' | 'κ' => 'k',
        'м' | 'μ' => 'm',
        'о' | 'ο' | 'σ' => 'o',
        'р' | 'ρ' => 'p',
        'т' | 'τ' => 't',
        'у' | 'υ' => 'y',
        'х' | 'χ' => 'x',
        'ν' => 'v',
        'ѕ' => 's',
        _ => return None,
    })
}

/// Símbolos que también son puntuación: solo valen como letra si les sigue
/// otra letra/dígito ("p!to" sí, "hola!" no).
fn is_ambiguous_symbol(c: char) -> bool {
    matches!(c, '!' | '¡' | '|' | '$' | '+')
}

fn leet(c: char) -> Option<char> {
    Some(match c {
        '0' => 'o',
        '1' | '!' | '¡' | '|' => 'i',
        '2' => 'z',
        '3' | '€' => 'e',
        '4' | '@' => 'a',
        '5' | '$' | '§' => 's',
        '6' | '9' => 'g',
        '7' | '+' => 't',
        '8' | 'ß' => 'b',
        '¢' | '©' => 'c',
        '®' => 'r',
        '×' => 'x',
        _ => return None,
    })
}

/// Convierte un carácter a 0..n letras base (ASCII minúscula u otras letras).
fn fold(c: char, out: &mut Vec<char>) {
    if let Some(h) = homoglyph(c).or_else(|| leet(c)) {
        out.push(h);
        return;
    }
    if c == WILDCARD {
        out.push(WILDCARD);
        return;
    }
    for k in std::iter::once(c).nfkc() {
        for d in std::iter::once(k).nfd() {
            if is_combining_mark(d) {
                continue;
            }
            if let Some(h) = homoglyph(d).or_else(|| leet(d)) {
                out.push(h);
            } else {
                out.extend(d.to_lowercase());
            }
        }
    }
}

/// Texto → secuencia de letras normalizadas, con fragmento y posición de origen.
fn normalize(text: &str) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::with_capacity(text.len());
    let mut token = 0usize;
    let mut buf = Vec::with_capacity(4);
    let mut it = text.char_indices().peekable();
    while let Some((start, c)) = it.next() {
        if is_invisible(c) {
            continue;
        }
        let end = start + c.len_utf8();
        buf.clear();
        if is_ambiguous_symbol(c) && !it.peek().is_some_and(|(_, n)| n.is_alphanumeric()) {
            // puntuación real → separador
        } else {
            fold(c, &mut buf);
        }
        for &ch in &buf {
            if ch.is_alphabetic() || ch == WILDCARD {
                units.push(Unit { ch, count: 1, start, end, token });
            } else if units.last().is_some_and(|u| u.token == token) {
                token += 1; // separador: cierra el fragmento actual
            }
        }
        if buf.is_empty() && units.last().is_some_and(|u| u.token == token) {
            token += 1;
        }
    }
    phonetic(&mut units);
    collapse_repeats(&mut units);
    units
}

/// Reglas fonéticas del español (afectan igual al texto y a la lista).
fn phonetic(u: &mut Vec<Unit>) {
    let next = |u: &[Unit], i: usize| u.get(i + 1).filter(|n| n.token == u[i].token).map(|n| n.ch);
    let mut out: Vec<Unit> = Vec::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        let mut x = u[i];
        let n = next(u, i);
        match x.ch {
            'c' if n == Some('h') => {
                x.ch = CH;
                x.end = u[i + 1].end;
                i += 1;
            }
            'h' if out.last().is_none_or(|p: &Unit| p.token != x.token || p.ch != 's') => {
                // h muda (salvo "sh", que sí suena: "shit" no debe volverse "sit").
                i += 1;
                continue;
            }
            'c' if matches!(n, Some('e' | 'i')) => x.ch = 's',
            'c' | 'q' | 'k' => {
                x.ch = 'k';
                // "que", "qui" → "ke", "ki"
                if u[i].ch == 'q' && n == Some('u') && matches!(u.get(i + 2).map(|z| z.ch), Some('e' | 'i')) {
                    x.end = u[i + 1].end;
                    i += 1;
                }
            }
            'g' if matches!(n, Some('e' | 'i')) => x.ch = 'j',
            'v' => x.ch = 'b',
            // En el chat mexicano "x" suele ser "ch" (xingada, xinga).
            'x' => x.ch = CH,
            'z' => x.ch = 's',
            'y' => x.ch = 'i',
            _ => {}
        }
        out.push(x);
        i += 1;
    }
    *u = out;
}

/// "piiiito" → "pito" (solo dentro del mismo fragmento).
fn collapse_repeats(u: &mut Vec<Unit>) {
    let mut out: Vec<Unit> = Vec::with_capacity(u.len());
    for x in u.drain(..) {
        match out.last_mut() {
            Some(prev) if prev.ch == x.ch && prev.token == x.token && x.ch != WILDCARD => {
                prev.end = x.end;
                prev.count = prev.count.saturating_add(x.count);
            }
            _ => out.push(x),
        }
    }
    *u = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Profanity {
        Profanity::new(DEFAULT_BAD_WORDS)
    }

    #[test]
    fn detecta_ofuscaciones_comunes() {
        let f = p();
        for t in [
            "pito", "pi to", "p1to", "P!T0", "p-i-t-o", "p.i.t.o", "p_i_t_o", "piiiiito", "PITOOO",
            "4buso", "a.buso", "a buso", "@bus0", "ABUSOS",
            "𝓹𝓲𝓽𝓸", "ｐｉｔｏ", "ⓟⓘⓣⓞ", "рito", "pі to",
            "pu\u{200B}to", "p*ta", "pútö",
            "v3rg4", "berga", "vergazo", "kabron", "kabrones", "cabrón", "perrra",
            "hijueputaaa", "pendejada", "PENDEJOS", "mi erda",
            "chingada", "xingada", "shit",
        ] {
            assert!(f.contains(t), "debió detectar: {t:?}");
        }
    }

    #[test]
    fn no_marca_palabras_normales() {
        let f = p();
        for t in [
            "computo", "disputa", "repito", "papi tomo", "capitolio", "pitón", "espíritu",
            "abusado de bueno", "la vergüenza", "culminar", "perrito lindo", "abuela",
            "hola amigos, buenas noches", "qué buena música", "**** wow ****", "a la una",
            "verdad", "pitufo", "mariachi", "socorro", "zorrillo",
            "una pera", "Sora", "nacional", "rapero", "sitio", "night", "@ana hola!",
        ] {
            assert!(!f.contains(t), "falso positivo: {t:?} -> {:?}", f.find(t));
        }
    }

    #[test]
    fn censura_solo_el_fragmento() {
        let f = p();
        assert_eq!(f.censor("eres un pi to jaja", "piip"), "eres un piip jaja");
        assert_eq!(f.censor("hola 4buso!", "piip"), "hola piip!");
    }

    #[test]
    fn lista_personalizada_con_prefijo() {
        let f = Profanity::new("tonto*\nfeo");
        assert!(f.contains("t0nt0te"));
        assert!(f.contains("f e o"));
        assert!(!f.contains("feote"));
        assert!(Profanity::new("").is_empty());
    }
}
