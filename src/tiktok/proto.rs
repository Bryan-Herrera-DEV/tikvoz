//! Esquema protobuf mínimo del webcast de TikTok LIVE.
//!
//! Solo declaramos los campos que usamos: protobuf ignora los desconocidos al
//! decodificar, así que no necesitamos el `.proto` completo (miles de líneas) ni
//! `protoc` en el build. Los números de campo vienen del esquema público que
//! mantiene la comunidad (TikTokLiveProto v3).

use prost::Message;

/// Sobre que viaja por el WebSocket en ambos sentidos.
#[derive(Clone, PartialEq, Message)]
pub struct PushFrame {
    #[prost(int64, tag = "1")]
    pub seq_id: i64,
    #[prost(int64, tag = "2")]
    pub log_id: i64,
    #[prost(int64, tag = "3")]
    pub service: i64,
    #[prost(int64, tag = "4")]
    pub method: i64,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PushHeader>,
    #[prost(string, tag = "6")]
    pub payload_encoding: String,
    #[prost(string, tag = "7")]
    pub payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    pub payload: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PushHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// Lote de mensajes (respuesta del fetch inicial y payload de cada frame "msg").
#[derive(Clone, PartialEq, Message)]
pub struct FetchResult {
    #[prost(message, repeated, tag = "1")]
    pub messages: Vec<BaseMessage>,
    #[prost(string, tag = "2")]
    pub cursor: String,
    #[prost(string, tag = "5")]
    pub internal_ext: String,
    #[prost(map = "string, string", tag = "7")]
    pub route_params: std::collections::HashMap<String, String>,
    #[prost(int64, tag = "8")]
    pub heartbeat_duration: i64,
    #[prost(bool, tag = "9")]
    pub need_ack: bool,
    #[prost(string, tag = "10")]
    pub push_server: String,
    #[prost(bool, tag = "11")]
    pub is_first: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct BaseMessage {
    /// Tipo de evento, p. ej. "WebcastChatMessage".
    #[prost(string, tag = "1")]
    pub method: String,
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
    #[prost(int64, tag = "3")]
    pub msg_id: i64,
    #[prost(bool, tag = "6")]
    pub is_history: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct HeartBeat {
    #[prost(int64, tag = "1")]
    pub room_id: i64,
    #[prost(int64, tag = "2")]
    pub send_packet_seq_id: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ImEnterRoom {
    #[prost(int64, tag = "1")]
    pub room_id: i64,
    #[prost(string, tag = "2")]
    pub room_tag: String,
    #[prost(string, tag = "3")]
    pub live_region: String,
    #[prost(int64, tag = "4")]
    pub live_id: i64,
    #[prost(string, tag = "5")]
    pub identity: String,
    #[prost(string, tag = "6")]
    pub cursor: String,
    #[prost(int64, tag = "7")]
    pub account_type: i64,
    #[prost(int64, tag = "8")]
    pub enter_unique_id: i64,
    #[prost(string, tag = "9")]
    pub filter_welcome_msg: String,
    #[prost(bool, tag = "10")]
    pub is_anchor_continue_keep_msg: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct User {
    #[prost(int64, tag = "1")]
    pub id: i64,
    #[prost(string, tag = "3")]
    pub nickname: String,
    /// El @handle.
    #[prost(string, tag = "38")]
    pub display_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ChatMessage {
    #[prost(message, optional, tag = "2")]
    pub user: Option<User>,
    #[prost(string, tag = "3")]
    pub content: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct GiftMessage {
    #[prost(int32, tag = "5")]
    pub repeat_count: i32,
    #[prost(message, optional, tag = "7")]
    pub user: Option<User>,
    #[prost(int32, tag = "9")]
    pub repeat_end: i32,
    #[prost(message, optional, tag = "15")]
    pub gift: Option<Gift>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Gift {
    #[prost(int64, tag = "5")]
    pub id: i64,
    /// 1 = regalo "combo" (llega repetido mientras dura la racha).
    #[prost(int32, tag = "11")]
    pub r#type: i32,
    #[prost(int32, tag = "12")]
    pub diamond_count: i32,
    #[prost(string, tag = "16")]
    pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct SocialMessage {
    #[prost(message, optional, tag = "2")]
    pub user: Option<User>,
    /// 1 = seguir, 3 = compartir.
    #[prost(int64, tag = "4")]
    pub action: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_roundtrip_ignora_campos_extra() {
        let chat = ChatMessage {
            user: Some(User {
                id: 7,
                nickname: "Ana".into(),
                display_id: "ana_01".into(),
            }),
            content: "hola".into(),
        };
        let mut bytes = chat.encode_to_vec();
        // Campo desconocido (tag 99, varint) como los que manda TikTok.
        bytes.extend_from_slice(&[0x98, 0x06, 0x01]);
        let back = ChatMessage::decode(bytes.as_slice()).unwrap();
        assert_eq!(back, chat);
    }
}
