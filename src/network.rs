//! Cached private interface candidates. Discovery answers using the peer's
//! subnet rather than an unrelated Internet/VPN route.
use crate::system::PrimaryNetworkInfo;
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

type Cache = Option<(Instant, Vec<PrimaryNetworkInfo>)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub fn candidates() -> Vec<PrimaryNetworkInfo> {
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some((at, interfaces)) = &*cache {
        if at.elapsed() < Duration::from_secs(5) {
            return interfaces.clone();
        }
    }
    let mut result = Vec::new();
    for interface in NetworkInterface::show().unwrap_or_default() {
        if interface.internal {
            continue;
        }
        for address in &interface.addr {
            let Addr::V4(address) = address else {
                continue;
            };
            if !address.ip.is_private() || address.ip.is_loopback() || address.ip.is_link_local() {
                continue;
            }
            result.push(PrimaryNetworkInfo {
                interface_name: interface.name.clone(),
                local_ip: address.ip.to_string(),
                mac_address: interface.mac_addr.clone(),
                subnet_mask: address.netmask.map(|n| n.to_string()),
                broadcast_address: address.broadcast.map(|n| n.to_string()),
                ping_address: Some(address.ip.to_string()),
                wol_port: 9,
            });
        }
    }
    result.sort_by_key(|n| {
        (
            virtual_adapter(&n.interface_name),
            n.interface_name.clone(),
            n.local_ip.clone(),
        )
    });
    *cache = Some((Instant::now(), result.clone()));
    result
}

pub fn primary() -> Option<PrimaryNetworkInfo> {
    candidates().into_iter().next()
}

pub fn for_peer(peer: IpAddr) -> Option<PrimaryNetworkInfo> {
    let IpAddr::V4(peer) = peer else {
        return None;
    };
    candidates().into_iter().find(|n| same_subnet(n, peer))
}

fn virtual_adapter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "virtual",
        "vmware",
        "vethernet",
        "vpn",
        "wireguard",
        "tailscale",
        "tun",
        "tap",
        "wsl",
    ]
    .iter()
    .any(|part| name.contains(part))
}

fn same_subnet(network: &PrimaryNetworkInfo, peer: Ipv4Addr) -> bool {
    let Ok(local) = network.local_ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let Some(mask) = network
        .subnet_mask
        .as_deref()
        .and_then(|m| m.parse::<Ipv4Addr>().ok())
    else {
        return false;
    };
    let mask = u32::from(mask);
    mask != 0 && (u32::from(peer) & mask) == (u32::from(local) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peer_selection_requires_same_private_subnet() {
        let network = PrimaryNetworkInfo {
            interface_name: "Ethernet".into(),
            local_ip: "10.0.0.19".into(),
            subnet_mask: Some("255.255.255.0".into()),
            mac_address: None,
            broadcast_address: None,
            ping_address: None,
            wol_port: 9,
        };
        assert!(same_subnet(&network, "10.0.0.50".parse().unwrap()));
        assert!(!same_subnet(&network, "10.0.1.50".parse().unwrap()));
        assert!(!same_subnet(&network, "8.8.8.8".parse().unwrap()));
        assert!(virtual_adapter("VMware Virtual Ethernet Adapter"));
        assert!(!virtual_adapter("Intel Ethernet"));
    }
}
