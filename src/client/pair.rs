use std::{collections::HashSet, net::SocketAddr};

use anyhow::Result;
use dialoguer::Confirm;
use mdns_sd::{ServiceDaemon, ServiceEvent};

pub async fn find_server_address(code: String) -> Result<Option<SocketAddr>> {
    let mdns = ServiceDaemon::new()?;

    let service_type = "_http._tcp.local.";
    let service_name = "wireless-display";
    let receiver = mdns.browse(service_type)?;
    println!("Browsing for '{}' on the local network...", service_name);

    let mut visited_servers = HashSet::new();

    while let Ok(event) = receiver.recv_async().await {
        if let ServiceEvent::ServiceResolved(info) = event {
            if !info.get_fullname().starts_with(service_name) {
                continue;
            }
            let properties = info.get_properties();

            if let Some(service_code) = properties.get("code") {
                if service_code.val_str() == code {
                    // get the port and address
                    let port = properties
                        .get("port")
                        .and_then(|p| p.val_str().parse::<u16>().ok());
                    let address = info.get_addresses().iter().find(|addr| addr.is_ipv4());

                    if let (Some(port), Some(address)) = (port, address) {
                        let ip_address = address.to_ip_addr();
                        if visited_servers.contains(&ip_address) {
                            continue;
                        }
                        visited_servers.insert(ip_address);

                        if Confirm::new()
                            .with_prompt(format!(
                                "Found server '{}' at {}. Connect?",
                                info.get_fullname(),
                                address
                            ))
                            .default(true)
                            .interact()?
                        {
                            mdns.stop_browse(service_type)?;
                            return Ok(Some(SocketAddr::new(ip_address, port)));
                        }
                    }
                }
            }
        }
    }

    mdns.stop_browse(service_type)?;

    Ok(None)
}
