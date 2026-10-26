use std::{
    ffi::OsStr,
    net::{Ipv4Addr, UdpSocket},
    path::Path,
    process::Command,
};

#[cfg(target_os = "windows")]
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr,
    sync::Mutex,
};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, FreeLibrary, GetLastError, LocalFree, ERROR_ALREADY_EXISTS,
        ERROR_FILE_NOT_FOUND, ERROR_NOT_ALL_ASSIGNED, ERROR_SUCCESS, HANDLE, LUID, WAIT_TIMEOUT,
    },
    Security::{
        AdjustTokenPrivileges,
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        GetTokenInformation, LookupPrivilegeValueW, TokenElevation, LUID_AND_ATTRIBUTES,
        SECURITY_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION,
        TOKEN_PRIVILEGES, TOKEN_QUERY,
    },
    System::{
        LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
        Power::SetSuspendState,
        Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Shutdown::LockWorkStation,
        SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW},
        Threading::{
            CreateEventW, CreateMutexW, GetCurrentProcess, GetCurrentProcessId, OpenEventW,
            OpenProcessToken, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        },
    },
    UI::WindowsAndMessaging::{
        MessageBoxW, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND,
        MB_TOPMOST, MB_YESNO, MESSAGEBOX_RESULT,
    },
};

#[cfg(target_os = "windows")]
const IDYES: MESSAGEBOX_RESULT = 6;

use crate::types::SystemAction;

#[cfg(target_os = "windows")]
const WINDOWS_RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
#[cfg(target_os = "windows")]
const WINDOWS_RUN_VALUE_NAME: &str = "WakeMATE Companion";
#[cfg(target_os = "windows")]
const UXTHEME_ORDINAL_SET_PREFERRED_APP_MODE: usize = 135;
#[cfg(target_os = "windows")]
const UXTHEME_ORDINAL_FLUSH_MENU_THEMES: usize = 136;

/// No runtime state may be read or written until this succeeds. An elevated
/// desktop process would turn ordinary phone input into administrator input.
pub fn ensure_normal_user_runtime() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let token = process_token(TOKEN_QUERY)?;
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0;
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(format!(
                "could not verify the Windows runtime token: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut session = 0;
        if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
            return Err(format!(
                "could not verify the Windows desktop session: {}",
                std::io::Error::last_os_error()
            ));
        }
        validate_runtime_context(elevation.TokenIsElevated != 0, session)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn validate_runtime_context(elevated: bool, session: u32) -> Result<(), String> {
    if elevated {
        return Err("WakeMATE must run as your normal Windows user. Close this copy and open WakeMATE from the Start menu without Run as administrator.".into());
    }
    if session == 0 {
        return Err("WakeMATE requires an interactive Windows user session; services and pre-login startup are unsupported.".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
struct OwnedWindowsHandle(HANDLE);

#[cfg(target_os = "windows")]
impl Drop for OwnedWindowsHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[cfg(target_os = "windows")]
fn process_token(access: u32) -> Result<OwnedWindowsHandle, String> {
    let mut handle = 0;
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut handle) } == 0 {
        return Err(format!(
            "could not inspect the Windows user token: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(OwnedWindowsHandle(handle))
}

/// A local installer may request an orderly exit only for the executable at
/// its exact installation path. It does not load credentials or open an API.
#[cfg(target_os = "windows")]
pub struct ExitSignal(OwnedWindowsHandle);

#[cfg(target_os = "windows")]
impl ExitSignal {
    pub fn create() -> Result<Self, String> {
        let name = current_exit_signal_name()?;
        // Only the owning user, SYSTEM, and administrators may stop this app.
        // Administrators need this narrow right for cross-user upgrades.
        let descriptor_text =
            wide_null("D:(A;;0x00100002;;;SY)(A;;0x00100002;;;BA)(A;;0x00100002;;;OW)");
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor_text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = unsafe { CreateEventW(&attributes, 1, 0, name.as_ptr()) };
        let error = unsafe { GetLastError() };
        unsafe { LocalFree(descriptor) };
        if handle == 0 {
            return Err(format!(
                "could not create the WakeMATE exit signal: {}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        let handle = OwnedWindowsHandle(handle);
        if error == ERROR_ALREADY_EXISTS {
            return Err("a WakeMATE exit signal already exists for this installation; close the other copy before starting".into());
        }
        Ok(Self(handle))
    }

    pub fn requested(&self) -> bool {
        // Fail closed if the event cannot be inspected.
        (unsafe { WaitForSingleObject((self.0).0, 0) }) != WAIT_TIMEOUT
    }
}

#[cfg(target_os = "windows")]
pub fn request_exit_for_current_executable() -> Result<(), String> {
    let name = current_exit_signal_name()?;
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if handle == 0 {
        let error = unsafe { GetLastError() };
        return if error == ERROR_FILE_NOT_FOUND {
            Ok(()) // No running 0.3+ instance at this path; never kill another copy.
        } else {
            Err(format!(
                "could not request WakeMATE exit: {}",
                std::io::Error::from_raw_os_error(error as i32)
            ))
        };
    }
    let handle = OwnedWindowsHandle(handle);
    if unsafe { SetEvent(handle.0) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn current_exit_signal_name() -> Result<Vec<u16>, String> {
    let path = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())?;
    Ok(wide_null(exit_signal_name(&path)))
}

#[cfg(target_os = "windows")]
fn exit_signal_name(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    // Match installer Pascal code exactly, independent of Unicode locale.
    let normalized = path.to_string_lossy().to_ascii_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let suffix: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("Global\\WakeMATE-Exit-{suffix}")
}

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

pub fn send_wol(mac: &str, broadcast: &str, port: u16) -> Result<(), String> {
    let mac = parse_mac(mac)?;
    let destination = wol_destination(broadcast, port)?;

    let mut packet = vec![0xFF; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac);
    }

    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
    socket
        .set_broadcast(true)
        .map_err(|error| error.to_string())?;
    socket
        .send_to(&packet, destination)
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn wol_destination(broadcast: &str, port: u16) -> Result<std::net::SocketAddrV4, String> {
    let ip: Ipv4Addr = broadcast
        .parse()
        .map_err(|_| "Wake destination must be an IPv4 address".to_string())?;
    if port == 0 || ip.is_unspecified() || ip.is_multicast() {
        return Err(
            "Wake destination must be unicast or broadcast with a nonzero port".to_string(),
        );
    }
    Ok(std::net::SocketAddrV4::new(ip, port))
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

fn parse_mac(mac: &str) -> Result<[u8; 6], String> {
    if !mac
        .chars()
        .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '-' | '.'))
    {
        return Err("MAC address contains invalid characters".to_string());
    }
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

fn run(program: impl AsRef<OsStr>, args: &[&str]) -> Result<(), String> {
    let mut command = Command::new(program.as_ref());
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let status = command.status().map_err(|error| error.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with status {status}",
            program.as_ref().to_string_lossy()
        ))
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
    format!("\"{}\" --installed", exe_path.display())
}

#[cfg(target_os = "windows")]
fn enable_preferred_dark_mode_windows() -> bool {
    let module = unsafe {
        LoadLibraryExW(
            wide_null("uxtheme.dll").as_ptr(),
            0,
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
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
            MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };

    result == IDYES
}

#[cfg(target_os = "windows")]
pub fn show_error_dialog(title: &str, message: &str) {
    unsafe {
        MessageBoxW(
            0,
            wide_null(message).as_ptr(),
            wide_null(title).as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
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
    // One owner across Fast User Switching / RDP sessions, because the TCP
    // listeners and controlled machine are shared. A second session must not
    // load different user credentials before discovering the bind conflict.
    let name = wide_null("Global\\WakeMATE-Companion-SingleInstance-BA6C6A3E");

    // Safety: `name` is a valid, null-terminated wide string kept alive for
    // the duration of this call, and CreateMutexW does not retain the
    // pointer past that.
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };

    if handle == 0 {
        return Err(format!(
            "could not acquire WakeMATE's machine-wide runtime ownership; another Windows user may own the companion: {}",
            std::io::Error::last_os_error()
        ));
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
        run(windows_directory(false)?.join("explorer.exe"), &[&path])
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
fn windows_directory(system: bool) -> Result<PathBuf, String> {
    // Read the trusted Windows directory from the OS, never PATH/SystemRoot.
    let mut buffer = vec![0_u16; 32768];
    let length = unsafe {
        if system {
            GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32)
        } else {
            GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32)
        }
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(format!(
            "could not resolve the Windows system directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..length]));
    if !path.is_absolute() {
        return Err("Windows returned a non-absolute system directory".into());
    }
    Ok(path)
}

#[cfg(target_os = "windows")]
struct ShutdownPrivilege {
    token: OwnedWindowsHandle,
    previous: TOKEN_PRIVILEGES,
}

#[cfg(target_os = "windows")]
impl ShutdownPrivilege {
    fn enable() -> Result<Self, String> {
        let token = process_token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY)?;
        let mut luid = LUID {
            LowPart: 0,
            HighPart: 0,
        };
        let name = wide_null("SeShutdownPrivilege");
        if unsafe { LookupPrivilegeValueW(ptr::null(), name.as_ptr(), &mut luid) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let requested = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut previous = TOKEN_PRIVILEGES {
            PrivilegeCount: 0,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: 0,
            }],
        };
        let mut returned = 0;
        let adjusted = unsafe {
            AdjustTokenPrivileges(
                token.0,
                0,
                &requested,
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                &mut returned,
            )
        };
        let error = unsafe { GetLastError() };
        if adjusted == 0 || error == ERROR_NOT_ALL_ASSIGNED {
            return Err(format!(
                "Windows user policy does not permit this sleep request: {}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        Ok(Self { token, previous })
    }
}

#[cfg(target_os = "windows")]
impl Drop for ShutdownPrivilege {
    fn drop(&mut self) {
        if self.previous.PrivilegeCount != 0
            && unsafe {
                AdjustTokenPrivileges(
                    self.token.0,
                    0,
                    &self.previous,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            } == 0
        {
            tracing::warn!("could not restore the original Windows shutdown privilege state");
        }
    }
}

#[cfg(target_os = "windows")]
fn sleep() -> Result<&'static str, String> {
    // AdjustTokenPrivileges can only enable an existing user right; it cannot
    // obtain administrator rights or grant a right denied by Windows policy.
    static POWER_OPERATION: Mutex<()> = Mutex::new(());
    let _operation = POWER_OPERATION
        .lock()
        .map_err(|_| "power controller is unavailable")?;
    let _privilege = ShutdownPrivilege::enable()?;
    if unsafe { SetSuspendState(0, 0, 0) } == 0 {
        return Err(format!(
            "Windows rejected sleep: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok("Windows accepted the sleep request")
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
    run(
        windows_directory(true)?.join("shutdown.exe"),
        &["/r", "/t", "0"],
    )?;
    Ok("restart command sent")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn restart() -> Result<&'static str, String> {
    run("shutdown", &["-r", "now"])?;
    Ok("restart command sent")
}

#[cfg(target_os = "windows")]
fn shutdown() -> Result<&'static str, String> {
    run(
        windows_directory(true)?.join("shutdown.exe"),
        &["/s", "/t", "0"],
    )?;
    Ok("shutdown command sent")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn shutdown() -> Result<&'static str, String> {
    run("shutdown", &["-h", "now"])?;
    Ok("shutdown command sent")
}

#[cfg(target_os = "windows")]
fn lock() -> Result<&'static str, String> {
    if unsafe { LockWorkStation() } == 0 {
        return Err(format!(
            "Windows rejected the lock request: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok("Windows accepted the lock request")
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
    run(windows_directory(true)?.join("shutdown.exe"), &["/l"])?;
    Ok("logoff command sent")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn logoff() -> Result<&'static str, String> {
    Err("logoff is not implemented for this platform yet".to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_mac, wol_destination};

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

    #[test]
    fn wake_validation_rejects_junk_hostnames_and_invalid_destinations() {
        assert!(parse_mac("00:11:22:33:44:55!!!").is_err());
        assert!(parse_mac("0011.2233.4455").is_ok());
        assert!(parse_mac("00-11-22-33-44-55").is_ok());
        assert!(wol_destination("example.com", 9).is_err());
        assert!(wol_destination("0.0.0.0", 9).is_err());
        assert!(wol_destination("224.0.0.1", 9).is_err());
        assert!(wol_destination("192.168.1.255", 0).is_err());
        assert!(wol_destination("192.168.1.255", 9).is_ok());
        assert!(wol_destination("255.255.255.255", 9).is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn startup_command_wraps_the_executable_path_in_quotes() {
        let command = startup_command(Path::new(
            r"C:\Program Files\WakeMATE\wakemate-companion.exe",
        ));
        assert_eq!(
            command,
            r#""C:\Program Files\WakeMATE\wakemate-companion.exe" --installed"#
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn runtime_policy_rejects_elevation_and_services() {
        assert!(super::validate_runtime_context(true, 1).is_err());
        assert!(super::validate_runtime_context(false, 0).is_err());
        assert!(super::validate_runtime_context(false, 1).is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn exit_signals_are_scoped_to_the_installed_path() {
        assert_ne!(
            super::exit_signal_name(Path::new(
                r"C:\Program Files\WakeMATE\wakemate-companion.exe"
            )),
            super::exit_signal_name(Path::new(r"C:\Projects\V2\wakemate-companion.exe")),
        );
        assert_eq!(
            super::exit_signal_name(Path::new(r"C:\Apps\WakeMATE.exe")),
            super::exit_signal_name(Path::new(r"c:\apps\wakemate.exe")),
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn graceful_exit_signal_round_trips_only_for_this_test_executable() {
        // The name is derived from this unit-test binary in target/, never
        // from a running or installed companion. No app/input/power is run.
        let signal = super::ExitSignal::create().unwrap();
        assert!(!signal.requested());
        super::request_exit_for_current_executable().unwrap();
        assert!(signal.requested());
    }
}
