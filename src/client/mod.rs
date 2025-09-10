use anyhow::Result;
use tokio::sync::mpsc;

mod connect;
mod gui;
mod pair;

#[derive(Debug, Clone)]
pub struct StreamFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub async fn run_cli_client(code: String, password: Option<String>) -> Result<()> {
    // find the server address and port using mDNS
    let server_addr = pair::find_server_address(code)
        .await?
        .ok_or(anyhow::anyhow!("Server not found"))?;

    let (frame_tx, frame_rx) = mpsc::channel::<StreamFrame>(10);

    // start the webrtc in a separate task
    let frame_tx_clone = frame_tx.clone();
    tokio::spawn(connect::start_webrtc(password, server_addr, frame_tx_clone));

    // run GUI in main thread
    if let Err(err) = gui::run_gui(frame_rx) {
        eprintln!("GUI error: {}", err);
    }

    Ok(())
}
