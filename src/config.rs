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
    pub username: String,
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

    pub skip_repeats: bool,
    pub repeat_window_secs: u64,
    pub filter: FilterConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Sistema,
            username: String::new(),
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
            skip_repeats: true,
            repeat_window_secs: 60,
            filter: FilterConfig::default(),
        }
    }
}

impl Settings {
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
}
