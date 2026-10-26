use crate::{
    config::SharedConfig,
    types::{DiscoveryResponse, PROTOCOL_VERSION},
};
use std::{
    collections::HashMap,
    io,
    net::IpAddr,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;

pub async fn bind(config: &SharedConfig) -> io::Result<UdpSocket> {
    let port = config
        .lock()
        .map_err(|_| io::Error::other("Configuration unavailable"))?
        .discovery_port;
    UdpSocket::bind(("0.0.0.0", port)).await
}

pub async fn run(config: SharedConfig, fingerprint: String, socket: UdpSocket) -> io::Result<()> {
    let mut buffer = [0u8; 1024];
    let mut budget = DiscoveryBudget::default();
    loop {
        let (len, peer) = socket.recv_from(&mut buffer).await?;
        let snapshot = config
            .lock()
            .map_err(|_| io::Error::other("Configuration unavailable"))?
            .clone();
        if !snapshot.discovery_enabled()
            || std::str::from_utf8(&buffer[..len]).ok().map(str::trim)
                != Some(snapshot.discovery_message.as_str())
        {
            continue;
        }
        let Some(network) = crate::network::for_peer(peer.ip()) else {
            continue;
        };
        if snapshot
            .preferred_address
            .as_ref()
            .is_some_and(|ip| *ip != network.local_ip)
        {
            continue;
        }
        if !budget.admit(peer.ip(), Instant::now()) {
            continue;
        }
        let response = DiscoveryResponse {
            device_name: snapshot.device_name.clone(),
            local_ip: network.local_ip,
            // MAC is delivered through the visual QR / authenticated info.
            mac_address: None,
            api_port: snapshot.bind_port(),
            tls_port: snapshot.tls_port,
            fp: fingerprint.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
        };
        let payload = serde_json::to_vec(&response).map_err(io::Error::other)?;
        socket.send_to(&payload, peer).await?;
    }
}

#[derive(Default)]
struct DiscoveryBudget {
    peers: HashMap<IpAddr, (Instant, u32)>,
    total: Option<(Instant, u32)>,
}
impl DiscoveryBudget {
    fn admit(&mut self, peer: IpAddr, now: Instant) -> bool {
        self.peers
            .retain(|_, (at, _)| now.duration_since(*at) < Duration::from_secs(1));
        let total = self.total.get_or_insert((now, 0));
        if now.duration_since(total.0) >= Duration::from_secs(1) {
            *total = (now, 0);
        }
        if total.1 >= 100 || (self.peers.len() >= 256 && !self.peers.contains_key(&peer)) {
            return false;
        }
        let entry = self.peers.entry(peer).or_insert((now, 0));
        if entry.1 >= 10 {
            return false;
        }
        entry.1 += 1;
        total.1 += 1;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_limits_each_peer_and_recovers_after_window() {
        let mut budget = DiscoveryBudget::default();
        let peer = "10.0.0.50".parse().unwrap();
        let now = Instant::now();
        for _ in 0..10 {
            assert!(budget.admit(peer, now));
        }
        assert!(!budget.admit(peer, now));
        assert!(budget.admit(peer, now + Duration::from_secs(2)));
        for host in 1..=100u8 {
            budget.admit(IpAddr::from([10, 0, 1, host]), now + Duration::from_secs(2));
        }
        assert!(!budget.admit("10.0.2.1".parse().unwrap(), now + Duration::from_secs(2)));
    }
}
