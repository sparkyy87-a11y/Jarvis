//! Findet andere Jarvis-Geräte im selben Netz per mDNS/Bonjour.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use jarvis_protocol::MDNS_SERVICE_TYPE;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::node::Node;

pub async fn run(node: Arc<Node>, port: u16) -> Result<()> {
    let mdns = ServiceDaemon::new()?;
    let host = gethostname::gethostname()
        .to_string_lossy()
        .replace(' ', "-");
    let instance = format!("jarvis-{}", node.me.id.simple());
    // Kein Gerätename im Netz (z. B. im Café-WLAN) – nur ID und öffentlicher Schlüssel.
    let props = [("id", node.me.id.to_string()), ("key", node.public_key())];
    let service = ServiceInfo::new(
        MDNS_SERVICE_TYPE,
        &instance,
        &format!("{host}.local."),
        "",
        port,
        &props[..],
    )?
    .enable_addr_auto();
    mdns.register(service)?;
    info!("Per mDNS sichtbar als {instance}");

    let events = mdns.browse(MDNS_SERVICE_TYPE)?;
    while let Ok(event) = events.recv_async().await {
        let ServiceEvent::ServiceResolved(info) = event else {
            continue;
        };
        let Some(id) = info
            .get_property_val_str("id")
            .and_then(|s| s.parse::<Uuid>().ok())
        else {
            continue;
        };
        // Nur mit eigenen (gekoppelten) Geräten verbinden – fremde Jarvis im Netz ignorieren.
        let trusted = info
            .get_property_val_str("key")
            .is_some_and(|k| node.is_trusted_key(k));
        // Nur eine Seite baut die Verbindung auf: die mit der kleineren ID.
        if !trusted || id == node.me.id || node.me.id > id || node.is_connected(&id) {
            continue;
        }
        let Some(ip) = info
            .get_addresses()
            .iter()
            .find(|ip| !ip.is_loopback())
            .copied()
        else {
            continue;
        };
        let addr = SocketAddr::new(ip, info.get_port());
        debug!("Eigenes Gerät gefunden unter {addr}");
        let node = node.clone();
        tokio::spawn(async move {
            if let Err(e) = node.connect(addr, None).await {
                warn!("Verbindung zu {addr} fehlgeschlagen: {e:#}");
            }
        });
    }
    Ok(())
}
