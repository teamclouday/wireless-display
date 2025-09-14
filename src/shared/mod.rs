use serde::{Deserialize, Serialize};

mod connect;

#[derive(Serialize, Deserialize)]
pub struct SdpData {
    pub sdp: String,
    pub password: Option<String>,
}

pub use connect::create_peer_connection;
