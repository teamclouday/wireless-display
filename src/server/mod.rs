use std::sync::Arc;

use anyhow::Result;
use dialoguer::Select;
use tokio::sync::Mutex;
use webrtc::{
    peer_connection::RTCPeerConnection,
    track::track_local::track_local_static_sample::TrackLocalStaticSample,
};
use windows_capture::monitor::Monitor;

mod capture;
mod route;

#[derive(PartialEq, Debug)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

pub struct AppState {
    pub screen_index: usize,
    pub password: Option<String>,
    pub connection: Mutex<ConnectionState>,
    pub peer_connection: Mutex<Option<Arc<RTCPeerConnection>>>,
    pub video_track: Mutex<Option<Arc<TrackLocalStaticSample>>>,
}

impl AppState {
    pub fn new(screen_index: usize, password: Option<String>) -> Self {
        AppState {
            screen_index,
            password,
            connection: Mutex::new(ConnectionState::Disconnected),
            peer_connection: Mutex::new(None),
            video_track: Mutex::new(None),
        }
    }
}

pub async fn run_cli_server(port: u16, password: Option<String>) -> Result<()> {
    // first select screen
    let monitors = Monitor::enumerate().unwrap();
    let selection = Select::new()
        .with_prompt("Select the virtual screen to use")
        .items(
            &monitors
                .iter()
                .map(|m| m.name().unwrap_or("Unknown".to_string()))
                .collect::<Vec<String>>(),
        )
        .default(0)
        .interact()?;

    // init app state
    let state = Arc::new(AppState::new(selection + 1, password.clone()));

    // prepare warp route
    let route = route::build_route(state.clone()).await?;

    // start screen capture
    tokio::spawn(capture::capture_screen(state.clone()));

    // start warp server
    println!("Starting server on port {}", port);
    warp::serve(route).run(([0, 0, 0, 0], port)).await;

    Ok(())
}
