use std::{
    net::{Ipv4Addr, UdpSocket},
    path::Path,
    process::Command,
};

use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};

use crate::types::SystemAction;

const DEFAULT_WOL_PORT: u16 = 9;

#[derive(Debug, Clone)]
pub struct PrimaryNetworkInfo {
    pub interface_name: String,
    pub local_ip: String,
    pub mac_address: Option<String>,
    pub subnet_mask: Option<String>,
    pub broadcast_address: Option<String>,
    pub ping_address: Option<String>,
    pub wol_port: u16,
}

pub fn local_ipv4() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;

    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(address) => Some(address.ip().to_string()),
        std::net::SocketAddr::V6(_) => None,
    }
}

pub fn primary_network_info() -> Option<PrimaryNetworkInfo> {
    let local_ip = local_ipv4()?;
    let local_ip_addr: Ipv4Addr = local_ip.parse().ok()?;
    let interfaces = NetworkInterface::show().ok()?;

    interfaces
        .into_iter()
        .find_map(|interface| interface_to_primary_info(interface, local_ip_addr))
}

pub fn send_wol(mac: &str, broadcast: &str, port: u16) -> Result<(), String> {
    let mac = parse_mac(mac)?;

    let mut packet = vec![0xFF; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac);
    }

    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
    socket
        .set_broadcast(true)
        .map_err(|error| error.to_string())?;
    socket
        .send_to(&packet, format!("{broadcast}:{port}"))
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn interface_to_primary_info(
    interface: NetworkInterface,
    local_ip: Ipv4Addr,
) -> Option<PrimaryNetworkInfo> {
    let matched_v4 = interface.addr.iter().find_map(|address| match address {
        Addr::V4(details) if details.ip == local_ip => Some(details),
        _ => None,
    })?;

    Some(PrimaryNetworkInfo {
        interface_name: interface.name,
        local_ip: matched_v4.ip.to_string(),
        mac_address: interface.mac_addr.filter(|value| !value.is_empty()),
        subnet_mask: matched_v4.netmask.map(|value| value.to_string()),
        broadcast_address: matched_v4.broadcast.map(|value| value.to_string()),
        ping_address: Some(matched_v4.ip.to_string()),
        wol_port: DEFAULT_WOL_PORT,
    })
}

pub fn perform_system_action(action: SystemAction) -> Result<&'static str, String> {
    match action {
        SystemAction::Sleep => sleep(),
        SystemAction::Restart => restart(),
        SystemAction::Shutdown => shutdown(),
        SystemAction::Lock => lock(),
        SystemAction::Logoff => logoff(),
    }
}

fn parse_mac(mac: &str) -> Result<[u8; 6], String> {
    let cleaned: String = mac
        .chars()
        .filter(|value| value.is_ascii_hexdigit())
        .collect();
    if cleaned.len() != 12 {
        return Err("MAC address must contain exactly 12 hex digits".to_string());
    }

    let mut bytes = [0_u8; 6];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&cleaned[start..start + 2], 16)
            .map_err(|_| "MAC address contains invalid hex digits".to_string())?;
    }

    Ok(bytes)
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| error.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with status {status}"))
    }
}

pub fn open_path(path: &Path) -> Result<(), String> {
    let path = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string();

    #[cfg(target_os = "windows")]
    {
        run("explorer.exe", &[&path])
    }

    #[cfg(target_os = "macos")]
    {
        run("open", &[&path])
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        run("xdg-open", &[&path])
    }
}

#[cfg(target_os = "windows")]
fn sleep() -> Result<&'static str, String> {
    run("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"])?;
    Ok("sleep command sent")
}

#[cfg(target_os = "macos")]
fn sleep() -> Result<&'static str, String> {
    run("pmset", &["sleepnow"])?;
    Ok("sleep command sent")
}

#[cfg(target_os = "linux")]
fn sleep() -> Result<&'static str, String> {
    run("systemctl", &["suspend"])?;
    Ok("sleep command sent")
}

#[cfg(target_os = "windows")]
fn restart() -> Result<&'static str, String> {
    run("shutdown", &["/r", "/t", "0"])?;
    Ok("restart command sent")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn restart() -> Result<&'static str, String> {
    run("shutdown", &["-r", "now"])?;
    Ok("restart command sent")
}

#[cfg(target_os = "windows")]
fn shutdown() -> Result<&'static str, String> {
    run("shutdown", &["/s", "/t", "0"])?;
    Ok("shutdown command sent")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn shutdown() -> Result<&'static str, String> {
    run("shutdown", &["-h", "now"])?;
    Ok("shutdown command sent")
}

#[cfg(target_os = "windows")]
fn lock() -> Result<&'static str, String> {
    run("rundll32.exe", &["user32.dll,LockWorkStation"])?;
    Ok("lock command sent")
}

#[cfg(target_os = "linux")]
fn lock() -> Result<&'static str, String> {
    run("loginctl", &["lock-session"])?;
    Ok("lock command sent")
}

#[cfg(target_os = "macos")]
fn lock() -> Result<&'static str, String> {
    Err("lock is not implemented for macOS yet".to_string())
}

#[cfg(target_os = "windows")]
fn logoff() -> Result<&'static str, String> {
    run("shutdown", &["/l"])?;
    Ok("logoff command sent")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn logoff() -> Result<&'static str, String> {
    Err("logoff is not implemented for this platform yet".to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_mac;

    #[test]
    fn parses_colon_separated_mac() {
        let parsed = parse_mac("00:11:22:33:44:55").unwrap();
        assert_eq!(parsed, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
    }

    #[test]
    fn rejects_invalid_mac() {
        let parsed = parse_mac("invalid");
        assert!(parsed.is_err());
    }
}
