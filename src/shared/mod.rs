use serde::{Deserialize, Serialize};

mod connect;
mod renderer;

#[derive(Serialize, Deserialize)]
pub struct SdpData {
    pub sdp: String,
    pub password: Option<String>,
}

pub use connect::create_peer_connection;
pub use renderer::{OpenGLRenderer, setup_opengl_context};
