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
//! 7. **Confusables (Unicode UTS #39)**: además de cirílico/griego, versalitas
//!    (`ᴘɪᴛᴏ`), letras de bandera (`🇵🇮🇹🇴`), letras en recuadro (`🅿🅸🆃🅾`),
//!    armenio e IPA.
//! 8. **Controles bidireccionales ("Trojan Source", CVE-2021-42574)**: con `U+202E`
//!    el texto se *muestra* al revés. Se ignoran y además se prueba el texto
//!    invertido; lo mismo con el texto "de cabeza" (`oʇᴉd`).
//! 9. **Rellenos invisibles**: Hangul filler (`ㅤ`), caracteres de etiqueta, etc.
//!
//! Sintaxis de la lista (inspirada en Discord AutoMod):
//! - una palabra por línea o separadas por coma;
//! - `palabra*` → todo lo que **empiece** así (`pendej*` → pendejo, pendejada…);
//! - `*palabra` → todo lo que **termine** así;
//! - `*palabra*` → la palabra **en cualquier parte** (cuidado: más falsos positivos);
//! - sin `*` → la palabra exacta y su plural (`-s`, `-es`);
//! - frases con espacios (`hijo de puta`) se detectan aunque las palabras sean largas;
//! - `[categoria]` en una línea agrupa lo que sigue (se activa/desactiva por grupo,
//!   como las categorías de Twitch AutoMod).
//!
//! La **lista de excepciones** se aplica por coincidencia, no por mensaje: una
//! palabra permitida no "desactiva" el filtro para el resto del mensaje (un fallo
//! conocido de Discord AutoMod).
//!
//! En el texto, un `*` funciona como comodín de una letra (`p*ta`).
//!
//! Lo que NO se hace a propósito: detectar letras desordenadas ("ptio"). Probado
//! contra el español, marca palabras normales (`carbón` ≈ `cabrón`): es el clásico
//! problema de Scunthorpe.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Lista por defecto, por categorías (español + algo de inglés). Editable desde la app.
pub const DEFAULT_BAD_WORDS: &str = "\
[insultos]
puta, puto, putita, putito, putamadre*, putamare*, hdp, hijueput*, hijodeput*, hijoeput*,
hijo de puta, hija de puta, puta madre, pendej*, cabron*, culer*, marica*, maricon*, joto,
zorra, perra, idiota, imbecil, estupid*, gilipollas, malparid*, gonorrea, careverga, cojud*,
mamaguev*, mamahuev*, conchatumadre*, conchetumadre*, conchesumadre*, concha tu madre,
concha de tu madre, chinga tu madre
[vulgar]
verga*, pito, polla, pinga, mierda*, chinga*, culo, coño, follar
[sexual]
violar*, violacion, violador*, abuso, pedofil*, porno*
[odio]
nazi, sudaca*, negrata*, sidoso*, nigger*, nigga*, faggot*
[violencia]
matate, suicidate, muerete, te voy a matar, ojala te mueras
[abreviaturas]
ptm, ptmr, ctm, ctmr, csm, alv, vrg, vrga, mrd, mrda, hpta, hpt, pndj*, mmgv
[ingles]
fuck*, motherfuck*, shit*, bitch*, cunt*, whore*, rape, rapist*";

/// Lista por defecto de la v0.2 (sin categorías). Se usa para migrar ajustes
/// guardados: si el usuario no la había tocado, pasa a la nueva automáticamente.
pub const DEFAULT_BAD_WORDS_V1: &str = "\
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
    /// Categoría de la regla (`[insultos]`, …) o "general".
    pub category: String,
}

#[derive(Debug, Clone)]
struct Rule {
    /// Letras normalizadas con cuántas veces se repiten seguidas ("perra" → r×2).
    pattern: Vec<(char, u8)>,
    /// `palabra*`: puede seguir cualquier cosa.
    prefix: bool,
    /// `*palabra`: puede empezar a mitad de una palabra.
    suffix: bool,
    /// Frase con espacios: no aplica la heurística de fragmentos cortos.
    multiword: bool,
    label: String,
    category: String,
}

/// Lista compilada. Construirla cuesta algo; se reconstruye solo si la lista cambia.
#[derive(Debug, Clone, Default)]
pub struct Profanity {
    rules: Vec<Rule>,
    /// Excepciones normalizadas (sin repeticiones ni separadores).
    allow: Vec<Vec<char>>,
}

const GENERAL: &str = "general";

/// Categorías presentes en una lista, en orden de aparición.
pub fn categories(list: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (cat, _) in entries(list) {
        if !out.contains(&cat) {
            out.push(cat);
        }
    }
    out
}

/// (categoría, entrada) de cada elemento de la lista.
fn entries(list: &str) -> Vec<(String, String)> {
    let mut category = GENERAL.to_string();
    let mut out = Vec::new();
    for line in list.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            category = line[1..line.len() - 1].trim().to_lowercase();
            continue;
        }
        for w in line.split([',', ';']).map(str::trim).filter(|w| !w.is_empty()) {
            out.push((category.clone(), w.to_string()));
        }
    }
    out
}

/// Letras normalizadas de una palabra/frase, sin separadores ni repeticiones.
fn letters(word: &str) -> Vec<char> {
    normalize(word).into_iter().map(|u| u.ch).filter(|c| *c != WILDCARD).collect()
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
    /// Atajo sin excepciones ni categorías apagadas (usado en tests).
    #[cfg(test)]
    pub fn new(list: &str) -> Self {
        Self::with_options(list, "", &[])
    }

    /// `allow`: excepciones (coma o línea). `disabled`: categorías apagadas.
    pub fn with_options(list: &str, allow: &str, disabled: &[String]) -> Self {
        let rules = entries(list)
            .into_iter()
            .filter(|(cat, _)| !disabled.iter().any(|d| d.eq_ignore_ascii_case(cat)))
            .filter_map(|(category, w)| {
                let prefix = w.ends_with('*');
                let suffix = w.starts_with('*');
                let base = w.trim_matches('*');
                let multiword = base.split_whitespace().nth(1).is_some();
                let pattern: Vec<(char, u8)> = normalize(base)
                    .into_iter()
                    .filter(|u| u.ch != WILDCARD)
                    .map(|u| (u.ch, u.count))
                    .collect();
                // Patrones de menos de 3 letras dan demasiados falsos positivos.
                (pattern.len() >= 3).then_some(Rule {
                    pattern,
                    prefix,
                    suffix,
                    multiword,
                    label: w,
                    category,
                })
            })
            .collect();
        let allow = allow
            .split([',', '\n', ';'])
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(letters)
            .filter(|l| !l.is_empty())
            .collect();
        Self { rules, allow }
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
        let hits = self.find_in(text);
        if !hits.is_empty() {
            return hits;
        }
        // Texto que se *ve* distinto de como está escrito: lo leemos como se ve.
        for alt in alternate_readings(text) {
            if let Some(h) = self.find_in(&alt).into_iter().next() {
                return vec![Hit { start: 0, end: text.len(), ..h }];
            }
        }
        Vec::new()
    }

    fn find_in(&self, text: &str) -> Vec<Hit> {
        let units = normalize(text);
        let mut hits = Vec::new();
        let mut i = 0;
        while i < units.len() {
            let token_start = i == 0 || units[i - 1].token != units[i].token;
            if let Some((end, rule)) = self.match_at(&units, i, token_start) {
                let start = if token_start { i } else { token_begin(&units, i) };
                if !self.is_allowed(&units, start, end) {
                    hits.push(Hit {
                        start: units[start].start,
                        end: units[end - 1].end,
                        rule: rule.label.clone(),
                        category: rule.category.clone(),
                    });
                    i = end;
                    continue;
                }
            }
            i += 1;
        }
        hits
    }

    /// ¿Lo detectado (extendido a palabras completas) está en las excepciones?
    fn is_allowed(&self, u: &[Unit], start: usize, end: usize) -> bool {
        if self.allow.is_empty() {
            return false;
        }
        let s = token_begin(u, start);
        let mut e = end;
        while e < u.len() && u[e].token == u[e - 1].token {
            e += 1;
        }
        let covered: Vec<char> = u[s..e].iter().map(|x| x.ch).collect();
        self.allow.contains(&covered)
    }

    /// Intenta cada regla en `i`; devuelve el índice final (exclusivo).
    fn match_at<'a>(&'a self, u: &[Unit], i: usize, token_start: bool) -> Option<(usize, &'a Rule)> {
        // Preferimos la coincidencia más larga (p. ej. "putamadre" sobre "puta").
        let mut best: Option<(usize, &Rule)> = None;
        for rule in self.rules.iter().filter(|r| token_start || r.suffix) {
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

    if !rule.multiword && !plausible_split(u, i, end) {
        return None;
    }
    Some(end)
}

/// Índice de la primera letra del fragmento que contiene `i`.
fn token_begin(u: &[Unit], mut i: usize) -> usize {
    while i > 0 && u[i - 1].token == u[i].token {
        i -= 1;
    }
    i
}

/// Lecturas alternativas de textos que se muestran distinto de como se escriben.
fn alternate_readings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Override bidireccional: lo que sigue se muestra de derecha a izquierda.
    if text.chars().any(|c| matches!(c, '\u{202E}' | '\u{202B}' | '\u{2067}')) {
        out.push(text.chars().rev().collect());
    }
    // Texto "de cabeza" (ɐ, ǝ, ʇ…): se lee al revés y con cada letra girada.
    if text.chars().any(|c| "ɐǝɔɟƃɥᴉɾʞɯɹʇʌʍʎ".contains(c)) {
        out.push(text.chars().rev().map(unflip).collect());
    }
    out
}

/// Letra "de cabeza" → letra normal.
fn unflip(c: char) -> char {
    match c {
        'ɐ' => 'a',
        'q' => 'b',
        'ɔ' => 'c',
        'p' => 'd',
        'ǝ' => 'e',
        'ɟ' => 'f',
        'ƃ' => 'g',
        'ɥ' => 'h',
        'ᴉ' | 'ı' => 'i',
        'ɾ' => 'j',
        'ʞ' => 'k',
        'ɯ' => 'm',
        'u' => 'n',
        'd' => 'p',
        'b' => 'q',
        'ɹ' => 'r',
        'ʇ' => 't',
        'n' => 'u',
        'ʌ' => 'v',
        'ʍ' => 'w',
        'ʎ' => 'y',
        '¡' => '!',
        '¿' => '?',
        other => other,
    }
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
        // controles bidireccionales (Trojan Source)
        || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{061C}')
        // rellenos Hangul y Khmer que se ven como nada (muy usados en nicks "vacíos")
        || matches!(c, '\u{3164}' | '\u{FFA0}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}')
        // caracteres de etiqueta
        || ('\u{E0000}'..='\u{E007F}').contains(&c)
        || is_combining_mark(c)
        || ('\u{FE00}'..='\u{FE0F}').contains(&c)
}

/// Letras de bandera (🇦), en círculo negro (🅐) y en recuadro negro (🅰): NFKC no las convierte.
fn enclosed_letter(c: char) -> Option<char> {
    let base = match c {
        '\u{1F1E6}'..='\u{1F1FF}' => 0x1F1E6,
        '\u{1F150}'..='\u{1F169}' => 0x1F150,
        '\u{1F170}'..='\u{1F189}' => 0x1F170,
        _ => return None,
    };
    char::from_u32('a' as u32 + (c as u32 - base))
}

fn homoglyph(c: char) -> Option<char> {
    if let Some(l) = enclosed_letter(c) {
        return Some(l);
    }
    Some(match c {
        // versalitas (NFKC no las convierte)
        'ᴀ' => 'a',
        'ʙ' => 'b',
        'ᴄ' => 'c',
        'ᴅ' => 'd',
        'ᴇ' => 'e',
        'ꜰ' => 'f',
        'ɢ' => 'g',
        'ʜ' => 'h',
        'ɪ' => 'i',
        'ᴊ' => 'j',
        'ᴋ' => 'k',
        'ʟ' => 'l',
        'ᴍ' => 'm',
        'ɴ' => 'n',
        'ᴏ' => 'o',
        'ᴘ' => 'p',
        'ǫ' => 'q',
        'ʀ' => 'r',
        'ꜱ' => 's',
        'ᴛ' => 't',
        'ᴜ' => 'u',
        'ᴠ' => 'v',
        'ᴡ' => 'w',
        'ʏ' => 'y',
        'ᴢ' => 'z',
        // IPA, armenio y cirílico extra (subconjunto de confusables.txt de UTS #39)
        'ɑ' => 'a',
        'ɡ' | 'ց' => 'g',
        'ɩ' => 'i',
        'ԁ' => 'd',
        'ԛ' => 'q',
        'ԝ' => 'w',
        'ӏ' => 'l',
        'օ' => 'o',
        'ս' => 'u',
        'ո' => 'n',
        'հ' => 'h',
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
    fn detecta_trucos_unicode_avanzados() {
        let f = p();
        for t in [
            "ᴘɪᴛᴏ",                 // versalitas
            "🇵🇮🇹🇴",               // letras de bandera
            "🅿🅸🆃🅾",              // recuadro negro
            "🅟🅘🅣🅞",              // círculo negro
            "\u{202E}otip",         // override RTL: se ve "pito"
            "oʇᴉd",                 // de cabeza
            "pu\u{3164}to",         // relleno Hangul invisible
            "pu\u{E0020}to",        // carácter de etiqueta
        ] {
            assert!(f.contains(t), "debió detectar: {t:?}");
        }
    }

    #[test]
    fn frases_abreviaturas_y_categorias() {
        let f = p();
        for t in ["hijo de puta", "la concha de tu madre", "te voy a matar", "ptm", "alv", "CTM wey"] {
            assert!(f.contains(t), "debió detectar: {t:?}");
        }
        for t in ["Álvaro", "hijo de Pedro", "voy a matar el tiempo", "carbón", "cosmos"] {
            assert!(!f.contains(t), "falso positivo: {t:?} -> {:?}", f.find(t));
        }
        assert_eq!(f.find("pito").first().map(|h| h.category.as_str()), Some("vulgar"));
        let sin_vulgar = Profanity::with_options(DEFAULT_BAD_WORDS, "", &["vulgar".into()]);
        assert!(!sin_vulgar.contains("pito"));
        assert!(sin_vulgar.contains("puto"));
        assert_eq!(
            categories(DEFAULT_BAD_WORDS),
            ["insultos", "vulgar", "sexual", "odio", "violencia", "abreviaturas", "ingles"]
        );
    }

    #[test]
    fn comodines_estilo_discord() {
        let f = Profanity::new("*ota, *gato*");
        assert!(f.contains("idiota"));
        assert!(!f.contains("otario"));
        assert!(f.contains("supergatote"));
        assert_eq!(f.censor("mi supergatote", "piip"), "mi piip");
    }

    #[test]
    fn excepciones_por_coincidencia_no_por_mensaje() {
        let f = Profanity::with_options("culo, puto", "culo", &[]);
        assert!(!f.contains("qué culo"));
        // La excepción no apaga el filtro para el resto del mensaje.
        assert!(f.contains("culo puto"));
        assert_eq!(f.censor("culo puto", "piip"), "culo piip");
    }

    #[test]
    fn lista_v1_sigue_detectando_lo_mismo() {
        let v1 = Profanity::new(DEFAULT_BAD_WORDS_V1);
        let v2 = p();
        for t in ["pito", "4buso", "hijueputa", "kabron", "shit", "nazi"] {
            assert!(v1.contains(t) && v2.contains(t), "{t}");
        }
    }

    #[test]
    fn lista_personalizada_con_prefijo() {
        let f = Profanity::new("tonto*\nfeo");
        assert!(f.contains("t0nt0te"));
        assert!(f.contains("f e o"));
        assert!(!f.contains("feote"));
        assert!(!Profanity::new("").contains("pito"));
    }
}
