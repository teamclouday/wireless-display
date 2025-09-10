use std::collections::HashMap;

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio::sync::broadcast;

pub async fn start_pairing_service(
    code: String,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let mdns = ServiceDaemon::new()?;

    let mut properties = HashMap::new();
    properties.insert("code".to_string(), code);

    let service_type = "_wireless-display._tcp.local.";
    let service_name = "wireless-display";

    let service_info = ServiceInfo::new(
        service_type,
        service_name,
        &format!("{}.local.", service_name),
        "",
        0,
        properties,
    )?;

    mdns.register(service_info)?;
    println!("Pairing service started. Advertised as '{}'", service_name);

    // wait for shutdown signal
    let _ = shutdown_rx.recv().await;

    mdns.shutdown()?;
    println!("Shutting down pairing service...");

    Ok(())
}
