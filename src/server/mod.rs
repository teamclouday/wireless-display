use std::sync::Arc;

use anyhow::Result;
use dialoguer::Select;
use tokio::sync::{Mutex, broadcast};
use webrtc::{
    data_channel::RTCDataChannel, peer_connection::RTCPeerConnection,
    track::track_local::track_local_static_sample::TrackLocalStaticSample,
};
use xcap::Monitor;

mod capture;
mod pair;
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
    pub mouse_channel: Mutex<Option<Arc<RTCDataChannel>>>,
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
            mouse_channel: Mutex::new(None),
        }
    }
}

pub async fn run_cli_server(
    port: u16,
    framerate: u32,
    code: String,
    password: Option<String>,
) -> Result<()> {
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

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

    // start screen capture
    let capture_screen_handle = tokio::spawn(capture::capture_screen(
        state.clone(),
        shutdown_tx.subscribe(),
    ));

    // start mouse capture
    let capture_mouse_handle = tokio::spawn(capture::capture_mouse(
        state.clone(),
        shutdown_tx.subscribe(),
    ));

    // start pairing service
    let pairing_handle = tokio::spawn(pair::start_pairing_service(
        port,
        code,
        shutdown_tx.subscribe(),
    ));

    // start warp server
    let route = route::create_warp_route(port, state.clone());
    warp::serve(route)
        .bind(([0, 0, 0, 0], port))
        .await
        .graceful(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .run()
        .await;

    println!("Shutting down...");

    let _ = shutdown_tx.send(());
    let shutdown_timeout = tokio::time::Duration::from_secs(3);
    let _ = tokio::time::timeout(shutdown_timeout, async {
        tokio::join!(capture_screen_handle, capture_mouse_handle, pairing_handle)
    })
    .await;

    Ok(())
}
