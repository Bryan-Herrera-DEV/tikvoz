//! Ajustes del usuario. Se guardan solos con la persistencia de eframe
//! (en Windows: %APPDATA%\TikVoz\data\app.ron).

use serde::{Deserialize, Serialize};

use crate::filter::FilterConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Sistema,
    Oscuro,
    Claro,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Muestra CPU/RAM/GPU del programa en la esquina inferior izquierda.
    pub show_monitor: bool,
    /// Usuario de TikTok.
    pub username: String,
    /// Canal de Kick (slug).
    pub kick_channel: String,
    /// API key opcional de Euler Stream (sube el límite de conexiones).
    pub api_key: String,
    pub auto_connect: bool,

    pub voice_id: Option<String>,
    pub rate: f32,
    pub volume: f32,
    /// Máximo de mensajes en espera; si se supera se descartan los más viejos.
    pub max_queue: usize,

    pub read_comments: bool,
    pub comment_template: String,
    pub read_gifts: bool,
    pub gift_template: String,
    /// Solo leer regalos que valgan al menos esto (diamantes por unidad).
    pub min_gift_diamonds: u32,
    pub read_follows: bool,
    pub follow_template: String,
    pub read_shares: bool,
    pub share_template: String,
    pub read_joins: bool,
    pub show_joins: bool,
    pub join_template: String,
    pub read_subs: bool,
    pub sub_template: String,
    pub gifted_sub_template: String,

    pub voice_per_user: bool,
    pub priority_gifts: bool,
    pub mods_priority: bool,
    pub only_followers: bool,
    /// Segundos mínimos entre lecturas del mismo usuario (0 = sin límite).
    pub user_cooldown_secs: u64,
    /// Usuarios separados por coma: siempre se leen y con prioridad.
    pub vip_users: String,

    pub skip_repeats: bool,
    pub repeat_window_secs: u64,
    pub filter: FilterConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Sistema,
            show_monitor: true,
            username: String::new(),
            kick_channel: String::new(),
            api_key: String::new(),
            auto_connect: false,
            voice_id: None,
            rate: 1.0,
            volume: 1.0,
            max_queue: 15,
            read_comments: true,
            comment_template: "{usuario} dice: {mensaje}".into(),
            read_gifts: true,
            gift_template: "{usuario} envió {cantidad} {regalo}".into(),
            min_gift_diamonds: 1,
            read_follows: false,
            follow_template: "{usuario} ahora te sigue".into(),
            read_shares: false,
            share_template: "{usuario} compartió el live".into(),
            read_joins: false,
            show_joins: false,
            join_template: "Bienvenido {usuario}".into(),
            read_subs: true,
            sub_template: "{usuario} se suscribió".into(),
            gifted_sub_template: "{usuario} regaló {cantidad} suscripciones".into(),
            voice_per_user: false,
            priority_gifts: true,
            mods_priority: true,
            only_followers: false,
            user_cooldown_secs: 0,
            vip_users: String::new(),
            skip_repeats: true,
            repeat_window_secs: 60,
            filter: FilterConfig::default(),
        }
    }
}

impl Settings {
    /// Adapta ajustes guardados por versiones anteriores.
    pub fn migrate(&mut self) {
        // v0.2 → v0.3: si la lista era la de fábrica, pasa a la nueva (superconjunto
        // con categorías, frases y abreviaturas). Si el usuario la editó, no se toca.
        if self.filter.bad_words.trim() == crate::profanity::DEFAULT_BAD_WORDS_V1.trim() {
            self.filter.bad_words = crate::profanity::DEFAULT_BAD_WORDS.into();
        }
        // v0.1 tenía "palabras bloqueadas" por substring; pasan al detector avanzado.
        let old = std::mem::take(&mut self.filter.blocked_words);
        for w in old.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            if !self
                .filter
                .bad_words
                .split([',', '\n'])
                .any(|b| b.trim().eq_ignore_ascii_case(w))
            {
                self.filter.bad_words.push_str(", ");
                self.filter.bad_words.push_str(w);
            }
        }
    }

    pub fn api_key(&self) -> Option<String> {
        let k = self.api_key.trim();
        (!k.is_empty()).then(|| k.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campos_faltantes_toman_valores_por_defecto() {
        // Simula un archivo de una versión anterior con menos campos.
        let s: Settings = serde_json::from_str(r#"{"username":"ana","rate":1.5}"#).unwrap();
        assert_eq!(s.username, "ana");
        assert_eq!(s.rate, 1.5);
        assert_eq!(s.max_queue, Settings::default().max_queue);
        assert_eq!(s.filter, FilterConfig::default());
    }

    #[test]
    fn migra_lista_de_fabrica_v1_y_respeta_la_editada() {
        let mut s = Settings::default();
        s.filter.bad_words = crate::profanity::DEFAULT_BAD_WORDS_V1.into();
        s.migrate();
        assert_eq!(s.filter.bad_words, crate::profanity::DEFAULT_BAD_WORDS);
        let mut e = Settings::default();
        e.filter.bad_words = "mi lista".into();
        e.migrate();
        assert_eq!(e.filter.bad_words, "mi lista");
    }

    #[test]
    fn migra_palabras_bloqueadas_viejas() {
        let mut s: Settings =
            serde_json::from_str(r#"{"filter":{"blocked_words":"spam, puto"}}"#).unwrap();
        s.migrate();
        assert!(s.filter.blocked_words.is_empty());
        assert!(s.filter.bad_words.ends_with(", spam"));
        assert_eq!(s.filter.bad_words.matches("puto").count(), 1);
    }
}
