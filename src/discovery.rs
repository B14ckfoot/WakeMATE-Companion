use tokio::net::UdpSocket;
use tracing::{error, warn};

use crate::{config::SharedConfig, system, types::DiscoveryResponse};

pub async fn run(config: SharedConfig) {
    let initial = match config.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            error!("failed to access config before starting discovery responder");
            return;
        }
    };

    let socket = match UdpSocket::bind(("0.0.0.0", initial.discovery_port)).await {
        Ok(socket) => socket,
        Err(error) => {
            error!(%error, port = initial.discovery_port, "failed to bind discovery socket");
            return;
        }
    };

    let mut buffer = [0_u8; 1024];

    loop {
        let (len, peer) = match socket.recv_from(&mut buffer).await {
            Ok(result) => result,
            Err(error) => {
                warn!(%error, "failed to receive discovery packet");
                continue;
            }
        };

        let snapshot = match config.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                warn!("failed to access config during discovery request");
                continue;
            }
        };

        if !snapshot.discovery_enabled() {
            continue;
        }

        let message = String::from_utf8_lossy(&buffer[..len]).trim().to_string();
        if message != snapshot.discovery_message {
            continue;
        }

        let network = system::primary_network_info();
        let api_port = snapshot.bind_port();
        let response = DiscoveryResponse {
            device_name: snapshot.device_name,
            local_ip: network
                .as_ref()
                .map(|info| info.local_ip.clone())
                .unwrap_or_else(|| system::local_ipv4().unwrap_or_else(|| "127.0.0.1".to_string())),
            mac_address: network.as_ref().and_then(|info| info.mac_address.clone()),
            api_port,
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let payload = match serde_json::to_vec(&response) {
            Ok(payload) => payload,
            Err(error) => {
                warn!(%error, "failed to serialize discovery response");
                continue;
            }
        };

        if let Err(error) = socket.send_to(&payload, peer).await {
            warn!(%error, ?peer, "failed to send discovery response");
        }
    }
}
