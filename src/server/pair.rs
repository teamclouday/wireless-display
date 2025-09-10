use std::collections::HashMap;

use anyhow::Result;
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use tokio::sync::broadcast;

pub async fn start_pairing_service(
    port: u16,
    code: String,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let mdns = ServiceDaemon::new()?;
    mdns.disable_interface(IfKind::IPv6)?;

    let mut properties = HashMap::new();
    properties.insert("code".to_string(), code);
    properties.insert("port".to_string(), port.to_string());

    let service_type = "_http._tcp.local.";
    let service_name = "wireless-display";

    let service_info = ServiceInfo::new(
        service_type,
        service_name,
        &format!("{}.local.", service_name),
        "",
        0,
        properties,
    )?
    .enable_addr_auto();

    mdns.register(service_info).map_err(|e| {
        eprintln!("Failed to register service: {}", e);
        e
    })?;
    println!("Pairing service started. Advertised as '{}'", service_name);

    // wait for shutdown signal
    let _ = shutdown_rx.recv().await;

    mdns.shutdown()?;
    println!("Shutting down pairing service...");

    Ok(())
}
