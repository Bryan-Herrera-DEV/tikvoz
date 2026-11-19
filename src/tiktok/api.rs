//! Llamadas HTTP previas al WebSocket:
//! 1. Resolver `@usuario` -> `room_id` (API pública de tiktok.com).
//! 2. Pedir al servidor de firmas (Euler Stream) la URL firmada del WebSocket
//!    y el primer lote de mensajes. Las firmas de TikTok son ofuscadas y cambian
//!    seguido, por eso todas las librerías de la comunidad delegan este paso.

use std::time::Duration;

use prost::Message;
use reqwest::{header, Client, StatusCode};
use serde::Deserialize;

use super::proto::FetchResult;
use super::LiveError;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const TIKTOK_ROOM_API: &str = "https://www.tiktok.com/api-live/user/room/";
const SIGN_API: &str = "https://api.eulerstream.com";
const CLIENT_NAME: &str = "ttlive-rust";

pub fn http_client() -> reqwest::Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .build()
}

#[derive(Deserialize)]
struct RoomResponse {
    #[serde(default)]
    message: String,
    data: Option<RoomData>,
}
#[derive(Deserialize)]
struct RoomData {
    user: Option<RoomUser>,
}
#[derive(Deserialize)]
struct RoomUser {
    #[serde(rename = "roomId", default)]
    room_id: String,
    /// 2 = en vivo, 4 = offline.
    #[serde(default)]
    status: i32,
}

/// Resuelve el `room_id` del live actual de un usuario. Si la API JSON falla
/// (TikTok a veces responde vacío cuando limita), cae al HTML de `/@usuario/live`.
pub async fn fetch_room_id(http: &Client, unique_id: &str) -> Result<i64, LiveError> {
    match fetch_room_id_api(http, unique_id).await {
        Err(LiveError::Other(reason)) => {
            log::debug!("API de room falló ({reason}), probando HTML");
            fetch_room_id_html(http, unique_id).await
        }
        other => other,
    }
}

async fn fetch_room_id_html(http: &Client, unique_id: &str) -> Result<i64, LiveError> {
    let html = http
        .get(format!("https://www.tiktok.com/@{unique_id}/live"))
        .send()
        .await
        .map_err(LiveError::net)?
        .text()
        .await
        .map_err(LiveError::net)?;
    extract_room_id_from_html(&html).ok_or(LiveError::Offline)
}

/// Busca el primer `"roomId":"<dígitos>"` no vacío en el HTML de la página del live.
fn extract_room_id_from_html(html: &str) -> Option<i64> {
    const KEY: &str = "\"roomId\":\"";
    html.match_indices(KEY).find_map(|(i, _)| {
        let rest = &html[i + KEY.len()..];
        let digits: &str = &rest[..rest.find('"')?];
        digits.parse::<i64>().ok().filter(|id| *id > 0)
    })
}

async fn fetch_room_id_api(http: &Client, unique_id: &str) -> Result<i64, LiveError> {
    let resp = http
        .get(TIKTOK_ROOM_API)
        .query(&[
            ("aid", "1988"),
            ("app_name", "tiktok_web"),
            ("device_platform", "web_pc"),
            ("uniqueId", unique_id),
            ("sourceType", "54"),
        ])
        .header(header::REFERER, "https://www.tiktok.com/")
        .send()
        .await
        .map_err(LiveError::net)?;

    let body: RoomResponse = resp
        .json()
        .await
        .map_err(|e| LiveError::Other(format!("respuesta inesperada de TikTok: {e}")))?;

    if body.message == "user_not_found" {
        return Err(LiveError::UserNotFound);
    }
    let user = body
        .data
        .and_then(|d| d.user)
        .ok_or_else(|| LiveError::Other("TikTok no devolvió datos del usuario (¿bloqueo temporal?)".into()))?;

    match user.room_id.parse::<i64>() {
        Ok(id) if id > 0 && user.status != 4 => Ok(id),
        _ => Err(LiveError::Offline),
    }
}

/// Respuesta del servidor de firmas: primer lote + cookies para el WebSocket.
pub struct SignedConnect {
    pub initial: FetchResult,
    pub cookies: String,
}

pub async fn fetch_signed_ws(
    http: &Client,
    room_id: i64,
    api_key: Option<&str>,
) -> Result<SignedConnect, LiveError> {
    let mut req = http
        .get(format!("{SIGN_API}/webcast/rooms/{room_id}/connect"))
        .query(&[
            ("client", CLIENT_NAME),
            ("client_enter", "true"),
            ("platform", "web"),
            ("user_agent", USER_AGENT),
        ]);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.header("X-Api-Key", key.trim());
    }
    let resp = req.send().await.map_err(LiveError::net)?;
    let status = resp.status();
    let cookies = resp
        .headers()
        .get("X-Set-TT-Cookie")
        .and_then(|v| v.to_str().ok())
        .map(parse_set_cookie)
        .unwrap_or_default();
    let bytes = resp.bytes().await.map_err(LiveError::net)?;

    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(LiveError::RateLimited(server_message(&bytes)));
    }
    if !status.is_success() {
        return Err(LiveError::Other(format!(
            "servidor de firmas respondió {status}: {}",
            server_message(&bytes)
        )));
    }
    if bytes.is_empty() {
        return Err(LiveError::Other("servidor de firmas devolvió respuesta vacía".into()));
    }
    if cookies.is_empty() {
        return Err(LiveError::Other("servidor de firmas no devolvió cookies".into()));
    }

    let initial = FetchResult::decode(bytes.as_ref())
        .map_err(|e| LiveError::Other(format!("protobuf inválido del servidor de firmas: {e}")))?;
    if initial.push_server.is_empty() {
        // Sin push_server TikTok no nos da WebSocket: normalmente el live ya terminó.
        return Err(LiveError::Offline);
    }
    Ok(SignedConnect { initial, cookies })
}

fn server_message(bytes: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Msg {
        message: Option<String>,
    }
    serde_json::from_slice::<Msg>(bytes)
        .ok()
        .and_then(|m| m.message)
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).into_owned())
}

/// Convierte un `Set-Cookie` (posiblemente con varias cookies) en un header `Cookie`.
pub fn parse_set_cookie(raw: &str) -> String {
    const ATTRS: [&str; 7] = ["path", "domain", "expires", "max-age", "secure", "httponly", "samesite"];
    raw.split([';', ','])
        .filter_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            let k = k.trim();
            if k.is_empty() || ATTRS.contains(&k.to_ascii_lowercase().as_str()) {
                None
            } else {
                Some(format!("{k}={}", v.trim()))
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_set_cookie_con_atributos() {
        let raw = "ttwid=1%7Cabc; Path=/; Domain=.tiktok.com; Expires=Wed, 21 Oct 2026 07:28:00 GMT; HttpOnly, \
                   msToken=xyz; Secure";
        assert_eq!(parse_set_cookie(raw), "ttwid=1%7Cabc; msToken=xyz");
    }

    #[test]
    fn extrae_room_id_del_html() {
        let html = r#"..."roomId":"","x":1,"roomId":"7710713113872829205","status":2"#;
        assert_eq!(extract_room_id_from_html(html), Some(7710713113872829205));
        assert_eq!(extract_room_id_from_html(r#""roomId":"""#), None);
    }
}
