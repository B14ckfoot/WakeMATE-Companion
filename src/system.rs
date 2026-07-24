use std::{
    ffi::OsStr,
    net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    path::Path,
    process::Command,
    time::Duration,
};

use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};

#[cfg(target_os = "windows")]
use std::{os::windows::ffi::OsStrExt, ptr};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, FreeLibrary, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE,
    },
    System::{
        LibraryLoader::{GetProcAddress, LoadLibraryW},
        Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        },
        Threading::CreateMutexW,
    },
    UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MESSAGEBOX_RESULT,
    },
};

#[cfg(target_os = "windows")]
const IDYES: MESSAGEBOX_RESULT = 6;

use crate::types::SystemAction;

const DEFAULT_WOL_PORT: u16 = 9;

#[cfg(target_os = "windows")]
const WINDOWS_RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
#[cfg(target_os = "windows")]
const WINDOWS_RUN_VALUE_NAME: &str = "WakeMATE Companion";
#[cfg(target_os = "windows")]
const WINDOWS_BOOT_TASK_NAME: &str = "WakeMATE Companion Server";
#[cfg(target_os = "windows")]
const UXTHEME_ORDINAL_SET_PREFERRED_APP_MODE: usize = 135;
#[cfg(target_os = "windows")]
const UXTHEME_ORDINAL_FLUSH_MENU_THEMES: usize = 136;

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
    route_probe_ipv4().or_else(first_interface_ipv4)
}

fn route_probe_ipv4() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;

    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(address) => Some(address.ip().to_string()),
        std::net::SocketAddr::V6(_) => None,
    }
}

/// Fallback when the routing probe cannot reach out (offline, VPN, or a
/// firewall blocking outbound UDP): pick a LAN-looking IPv4 straight from
/// the interface list so the pairing QR code still carries an address.
fn first_interface_ipv4() -> Option<String> {
    let interfaces = NetworkInterface::show().ok()?;
    let candidates: Vec<Ipv4Addr> = interfaces
        .iter()
        .flat_map(|interface| interface.addr.iter())
        .filter_map(|address| match address {
            Addr::V4(details)
                if !details.ip.is_loopback() && !details.ip.is_link_local() =>
            {
                Some(details.ip)
            }
            _ => None,
        })
        .collect();

    candidates
        .iter()
        .find(|ip| ip.is_private())
        .or_else(|| candidates.first())
        .map(|ip| ip.to_string())
}

pub fn primary_network_info() -> Option<PrimaryNetworkInfo> {
    let local_ip = local_ipv4()?;
    let local_ip_addr: Ipv4Addr = local_ip.parse().ok()?;
    let interfaces = NetworkInterface::show().ok()?;

    interfaces
        .into_iter()
        .find_map(|interface| interface_to_primary_info(interface, local_ip_addr))
}

pub fn server_is_reachable(bind_address: &str) -> bool {
    let probe = loopback_probe_address(bind_address);
    let Some(address) = probe.parse::<SocketAddr>().ok() else {
        return false;
    };

    TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok()
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

fn loopback_probe_address(bind_address: &str) -> String {
    if let Some(port) = bind_address.rsplit(':').next() {
        format!("127.0.0.1:{port}")
    } else {
        bind_address.to_string()
    }
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

pub fn sync_launch_on_startup(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        sync_launch_on_startup_windows(enabled)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = enabled;
        Ok(())
    }
}

pub fn enable_preferred_dark_mode() -> bool {
    #[cfg(target_os = "windows")]
    {
        enable_preferred_dark_mode_windows()
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub fn sync_prelogon_server_task(enabled: bool, config_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        sync_prelogon_server_task_windows(enabled, config_path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = enabled;
        let _ = config_path;
        Ok(())
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

#[cfg(target_os = "windows")]
fn sync_launch_on_startup_windows(enabled: bool) -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|error| error.to_string())?;
    let value_name = wide_null(WINDOWS_RUN_VALUE_NAME);
    let key = open_windows_run_key()?;

    let result = unsafe {
        if enabled {
            let command = wide_null(startup_command(&exe_path));
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr() as *const u8,
                (command.len() * std::mem::size_of::<u16>()) as u32,
            )
        } else {
            RegDeleteValueW(key, value_name.as_ptr())
        }
    };

    let close_result = unsafe { RegCloseKey(key) };
    if close_result != ERROR_SUCCESS {
        return Err(format!(
            "failed to close the WakeMATE startup registry key: error {close_result}"
        ));
    }

    if enabled {
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "failed to enable WakeMATE startup registration: error {result}"
            ))
        }
    } else if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!(
            "failed to disable WakeMATE startup registration: error {result}"
        ))
    }
}

#[cfg(target_os = "windows")]
fn open_windows_run_key() -> Result<HKEY, String> {
    let subkey = wide_null(WINDOWS_RUN_KEY_PATH);
    let mut key = 0;

    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };

    if result == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(format!(
            "failed to open the WakeMATE startup registry key: error {result}"
        ))
    }
}

#[cfg(target_os = "windows")]
fn startup_command(exe_path: &Path) -> String {
    format!("\"{}\"", exe_path.display())
}

#[cfg(target_os = "windows")]
fn sync_prelogon_server_task_windows(enabled: bool, config_path: &Path) -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|error| error.to_string())?;

    if enabled {
        let task_command = scheduled_task_command(&exe_path, config_path);
        run(
            "schtasks.exe",
            &[
                "/Create",
                "/TN",
                WINDOWS_BOOT_TASK_NAME,
                "/SC",
                "ONSTART",
                "/RU",
                "SYSTEM",
                "/RL",
                "HIGHEST",
                "/TR",
                &task_command,
                "/F",
            ],
        )
        .map_err(|error| format!("failed to create the WakeMATE boot task: {error}"))
    } else {
        match run(
            "schtasks.exe",
            &["/Delete", "/TN", WINDOWS_BOOT_TASK_NAME, "/F"],
        ) {
            Ok(()) => Ok(()),
            Err(error) if error.contains("ERROR: The system cannot find the file specified.") => {
                Ok(())
            }
            Err(error) => Err(format!("failed to delete the WakeMATE boot task: {error}")),
        }
    }
}

#[cfg(target_os = "windows")]
fn scheduled_task_command(exe_path: &Path, config_path: &Path) -> String {
    format!(
        "\"{}\" --headless-server --config-path \"{}\"",
        exe_path.display(),
        config_path.display()
    )
}

#[cfg(target_os = "windows")]
fn enable_preferred_dark_mode_windows() -> bool {
    let module = unsafe { LoadLibraryW(wide_null("uxtheme.dll").as_ptr()) };
    if module == 0 {
        return false;
    }

    let set_mode =
        unsafe { GetProcAddress(module, UXTHEME_ORDINAL_SET_PREFERRED_APP_MODE as *const u8) };
    let flush_menu_themes =
        unsafe { GetProcAddress(module, UXTHEME_ORDINAL_FLUSH_MENU_THEMES as *const u8) };

    let enabled = if let Some(set_mode) = set_mode {
        let set_mode: SetPreferredAppMode = unsafe { std::mem::transmute(set_mode) };
        let _ = unsafe { set_mode(PreferredAppMode::FORCE_DARK) };
        true
    } else {
        false
    };

    if let Some(flush_menu_themes) = flush_menu_themes {
        let flush_menu_themes: FlushMenuThemes = unsafe { std::mem::transmute(flush_menu_themes) };
        unsafe { flush_menu_themes() };
    }

    unsafe {
        FreeLibrary(module);
    }

    enabled
}

#[cfg(target_os = "windows")]
fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(target_os = "windows")]
#[repr(transparent)]
#[derive(Clone, Copy)]
struct PreferredAppMode(i32);

#[cfg(target_os = "windows")]
impl PreferredAppMode {
    const FORCE_DARK: Self = Self(2);
}

#[cfg(target_os = "windows")]
type SetPreferredAppMode = unsafe extern "system" fn(PreferredAppMode) -> PreferredAppMode;

#[cfg(target_os = "windows")]
type FlushMenuThemes = unsafe extern "system" fn();

/// Shows a native, modal, always-on-top Yes/No prompt. Returns `true` only
/// on an explicit "Yes".
///
/// This intentionally uses a plain Win32 `MessageBoxW` rather than a custom
/// window: it is unambiguous, always on top, cannot be scripted or spoofed
/// by web content, and needs no additional rendering code to get right for
/// a security-critical decision.
#[cfg(target_os = "windows")]
pub fn confirm_dialog(title: &str, message: &str) -> bool {
    let title = wide_null(title);
    let message = wide_null(message);

    let result = unsafe {
        MessageBoxW(
            0,
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };

    result == IDYES
}

/// Asks the person sitting at this computer whether a device should be
/// allowed to pair and gain input/power control.
#[cfg(target_os = "windows")]
pub fn confirm_pairing_dialog(device_hint: &str) -> bool {
    confirm_dialog(
        "WakeMATE Pairing Request",
        &format!(
            "A device at {device_hint} wants to pair with this computer and enable remote mouse, keyboard, and power control.\n\nOnly approve this if you just started pairing from the WakeMATE app on your own phone.\n\nAllow it?",
        ),
    )
}

/// Guards a Windows named mutex for the lifetime of the process so a second
/// copy of WakeMATE cannot bind the same port or show a duplicate tray icon.
/// Keep the returned guard alive for as long as the app runs; dropping it
/// releases the lock.
#[cfg(target_os = "windows")]
pub struct SingleInstanceLock {
    handle: HANDLE,
}

#[cfg(target_os = "windows")]
impl Drop for SingleInstanceLock {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

/// Attempts to acquire the single-instance lock. Returns `Ok(None)` if
/// another WakeMATE process already holds it.
#[cfg(target_os = "windows")]
pub fn acquire_single_instance_lock() -> Result<Option<SingleInstanceLock>, String> {
    let name = wide_null("Local\\WakeMATE-Companion-SingleInstance-BA6C6A3E");

    // Safety: `name` is a valid, null-terminated wide string kept alive for
    // the duration of this call, and CreateMutexW does not retain the
    // pointer past that.
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };

    if handle == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }

    let already_running =
        unsafe { windows_sys::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }

    Ok(Some(SingleInstanceLock { handle }))
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

    #[cfg(target_os = "windows")]
    use super::startup_command;
    #[cfg(target_os = "windows")]
    use std::path::Path;

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

    #[cfg(target_os = "windows")]
    #[test]
    fn startup_command_wraps_the_executable_path_in_quotes() {
        let command = startup_command(Path::new(
            r"C:\Program Files\WakeMATE\wakemate-companion.exe",
        ));
        assert_eq!(
            command,
            r#""C:\Program Files\WakeMATE\wakemate-companion.exe""#
        );
    }
}
