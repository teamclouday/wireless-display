use std::sync::Arc;

use anyhow::Result;
use dialoguer::Select;
use tokio::sync::Mutex;
use webrtc::{
    peer_connection::RTCPeerConnection,
    track::track_local::track_local_static_sample::TrackLocalStaticSample,
};
use xcap::Monitor;

mod capture;
mod route;

use capture::CaptureDevice;

#[derive(PartialEq, Debug)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

pub struct AppState {
    pub device: CaptureDevice,
    pub framerate: u32,
    pub password: Option<String>,
    pub connection: Mutex<ConnectionState>,
    pub peer_connection: Mutex<Option<Arc<RTCPeerConnection>>>,
    pub video_track: Mutex<Option<Arc<TrackLocalStaticSample>>>,
}

impl AppState {
    pub fn new(device: CaptureDevice, framerate: u32, password: Option<String>) -> Self {
        AppState {
            device,
            framerate,
            password,
            connection: Mutex::new(ConnectionState::Disconnected),
            peer_connection: Mutex::new(None),
            video_track: Mutex::new(None),
        }
    }
}

pub async fn run_cli_server(port: u16, framerate: u32, password: Option<String>) -> Result<()> {
    // first select screen
    let devices = Monitor::all()?
        .into_iter()
        .enumerate()
        .map(|(index, m)| CaptureDevice {
            index,
            name: m.name().unwrap_or("Unknown".to_string()),
            width: m.width().unwrap_or_default(),
            height: m.height().unwrap_or_default(),
            x: m.x().unwrap_or_default(),
            y: m.y().unwrap_or_default(),
        })
        .collect::<Vec<CaptureDevice>>();
    let device_index = Select::new()
        .with_prompt("Select the virtual screen to use")
        .items(
            &devices
                .iter()
                .map(|m| format!("{}. {}", m.index + 1, m))
                .collect::<Vec<String>>(),
        )
        .default(0)
        .interact()?;

    // init app state
    let state = Arc::new(AppState::new(
        devices[device_index].to_owned(),
        framerate,
        password.clone(),
    ));

    // prepare warp route
    let route = route::build_route(state.clone()).await;

    // start screen capture
    tokio::spawn(capture::capture_screen(state.clone()));

    // start warp server
    println!("Starting server on port {}", port);
    warp::serve(route).run(([0, 0, 0, 0], port)).await;

    Ok(())
}
