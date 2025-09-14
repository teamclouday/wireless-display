use serde::{Deserialize, Serialize};

mod connect;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SdpData {
    pub sdp: String,
    pub password: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MousePosition {
    pub x: f64,
    pub y: f64,
}

pub use connect::create_peer_connection;
