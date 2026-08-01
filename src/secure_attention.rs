//! Windows Secure Attention Sequence (Ctrl+Alt+Delete) support.
//!
//! Ctrl+Alt+Delete is the Windows *Secure Attention Sequence* (SAS), and
//! Windows deliberately makes it unspoofable: the real keystroke is claimed in
//! kernel mode and delivered straight to winlogon, while synthetic input
//! (`SendInput`, `keybd_event`, and therefore every userland automation crate,
//! including the `enigo` we use for ordinary keys) is filtered out of that
//! path. There is no privilege level at which faking the three keystrokes
//! works -- that is the entire point of the sequence, and defeating it would
//! be a security bypass rather than a feature.
//!
//! The one sanctioned path is `SendSAS` in `sas.dll`, which asks winlogon to
//! raise the sequence on the caller's behalf. Microsoft gates it on two
//! independent conditions:
//!
//! 1. **Caller privilege.** The process must be a Windows service, or a
//!    desktop app whose manifest sets `uiAccess="true"`. A `uiAccess` app must
//!    additionally be Authenticode-signed and live in a protected location
//!    (`\Program Files\` or `\Windows\System32\`), and UAC must be on.
//! 2. **Machine policy.** *Computer Configuration | Administrative Templates |
//!    Windows Components | Windows Logon Options | Disable or enable software
//!    Secure Attention Sequence* must permit it. That policy is the registry
//!    value `SoftwareSASGeneration` under
//!    `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System`:
//!    0 = none (the Windows default), 1 = services, 2 = Ease of Access
//!    (`uiAccess`) apps, 3 = both.
//!
//! WakeMATE ships as an ordinary interactive tray process, so on a stock
//! machine both conditions fail. We report that honestly instead of pretending
//! it worked.
//!
//! `SendSAS` returns `VOID` and reports nothing at all, so eligibility is
//! checked *before* calling it. Without that pre-check a refused call is
//! indistinguishable from a successful one -- which is precisely the
//! false-success bug this module exists to replace.

// The eligibility rule and its constants are consumed by `windows_impl`, but
// they stay compiled -- and unit-tested -- on every host so the logic can be
// verified from a macOS dev machine. Only the Windows build "uses" them.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

/// What the companion was able to do about a Secure Attention Sequence
/// request. Deliberately does not include a "probably worked" state: either
/// Windows accepted the request or we say why it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SasOutcome {
    /// Windows accepted the request and winlogon is raising the sequence.
    Requested,
    /// This computer can do it, but not from this process as configured.
    PermissionRequired { detail: String },
    /// This operating system has no Secure Attention Sequence at all.
    Unsupported { detail: String },
    /// Eligible, but the call itself failed.
    Failed { detail: String },
}

/// Machine policy value: software SAS generation is not permitted at all.
/// This is the Windows default.
pub const SAS_POLICY_NONE: u32 = 0;
/// Machine policy value: services may raise a software SAS.
pub const SAS_POLICY_SERVICES: u32 = 1;
/// Machine policy value: Ease of Access (`uiAccess`) apps may raise one.
pub const SAS_POLICY_EASE_OF_ACCESS: u32 = 2;
/// Machine policy value: both services and Ease of Access apps may.
pub const SAS_POLICY_BOTH: u32 = 3;

/// Why `SendSAS` may or may not be called, decided from the machine policy
/// and how this process is running. Split out from the Win32 calls so the
/// rule can be unit-tested on any platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SasEligibility {
    Allowed,
    /// The caller qualifies, but `SoftwareSASGeneration` does not allow it.
    PolicyDisabled,
    /// The policy would allow it, or does not matter yet, because this
    /// process is neither a service nor a `uiAccess` desktop app.
    CallerNotPrivileged,
}

/// Applies Microsoft's two-condition rule. A service needs policy 1 or 3; a
/// `uiAccess` desktop app needs policy 2 or 3.
///
/// WakeMATE's interactive tray host can only ever qualify through the
/// `uiAccess` branch. Production builds do not start a service-hosted API.
pub fn evaluate_eligibility(policy: u32, has_uiaccess: bool, is_service: bool) -> SasEligibility {
    if !is_service && !has_uiaccess {
        return SasEligibility::CallerNotPrivileged;
    }

    let permitted = if is_service {
        policy == SAS_POLICY_SERVICES || policy == SAS_POLICY_BOTH
    } else {
        policy == SAS_POLICY_EASE_OF_ACCESS || policy == SAS_POLICY_BOTH
    };

    if permitted {
        SasEligibility::Allowed
    } else {
        SasEligibility::PolicyDisabled
    }
}

/// Operator-facing explanation for a refusal. Kept next to
/// [`evaluate_eligibility`] so the wording and the rule cannot drift apart,
/// and deliberately free of anything host-specific -- it travels to the phone.
pub fn eligibility_detail(eligibility: SasEligibility) -> String {
    match eligibility {
        SasEligibility::Allowed => "Windows permits a software Secure Attention Sequence from this companion".to_string(),
        SasEligibility::CallerNotPrivileged => "Windows only accepts a software Ctrl+Alt+Delete from a service or a signed accessibility app installed in Program Files. The WakeMATE companion runs as a normal desktop app, so Windows refuses it.".to_string(),
        SasEligibility::PolicyDisabled => "Windows policy \"Disable or enable software Secure Attention Sequence\" is off on this computer, so no application may raise Ctrl+Alt+Delete.".to_string(),
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::request_secure_attention_sequence;

#[cfg(not(target_os = "windows"))]
pub fn request_secure_attention_sequence() -> SasOutcome {
    SasOutcome::Unsupported {
        detail: "The Secure Attention Sequence is a Windows feature; this companion is not running on Windows.".to_string(),
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, FreeLibrary, ERROR_SUCCESS, HANDLE},
        Security::{GetTokenInformation, TokenUIAccess, TOKEN_QUERY},
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD},
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };

    use super::{
        eligibility_detail, evaluate_eligibility, SasEligibility, SasOutcome, SAS_POLICY_NONE,
    };

    const SAS_POLICY_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System";
    const SAS_POLICY_VALUE: &str = "SoftwareSASGeneration";

    /// `VOID SendSAS(BOOL AsUser)` as exported by `sas.dll`. Resolved at run
    /// time rather than link time so the companion still starts on a Windows
    /// install that lacks the DLL instead of failing to load.
    type SendSasFn = unsafe extern "system" fn(i32);

    pub fn request_secure_attention_sequence() -> SasOutcome {
        // WakeMATE's tray host is a normal interactive process, so the
        // service branch of the rule is always false here.
        const IS_SERVICE: bool = false;

        let policy = software_sas_generation();
        let has_uiaccess = process_has_uiaccess();
        let eligibility = evaluate_eligibility(policy, has_uiaccess, IS_SERVICE);

        if eligibility != SasEligibility::Allowed {
            return SasOutcome::PermissionRequired {
                detail: eligibility_detail(eligibility),
            };
        }

        // AsUser is TRUE here because this process runs as the signed-in
        // user; a service host would pass FALSE.
        match invoke_send_sas(!IS_SERVICE) {
            Ok(()) => SasOutcome::Requested,
            Err(detail) => SasOutcome::Failed { detail },
        }
    }

    /// Reads the machine policy. A missing value means the policy was never
    /// configured, which Windows treats as "not permitted".
    fn software_sas_generation() -> u32 {
        let subkey = wide_null(SAS_POLICY_KEY);
        let value_name = wide_null(SAS_POLICY_VALUE);
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;

        // Safety: both wide strings are null-terminated and outlive the call,
        // and `data`/`size` describe a correctly sized DWORD buffer.
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_DWORD,
                ptr::null_mut(),
                &mut data as *mut u32 as *mut std::ffi::c_void,
                &mut size,
            )
        };

        if result == ERROR_SUCCESS {
            data
        } else {
            SAS_POLICY_NONE
        }
    }

    /// True when this process holds UIAccess, i.e. it was launched from a
    /// signed executable in a protected directory whose manifest requests it.
    fn process_has_uiaccess() -> bool {
        let mut token: HANDLE = 0;

        // Safety: `token` is a valid out-parameter; the handle is closed below
        // on every path where it was successfully opened.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }

        let mut ui_access: u32 = 0;
        let mut returned: u32 = 0;

        // Safety: TokenUIAccess yields a DWORD, which is what the buffer is.
        let queried = unsafe {
            GetTokenInformation(
                token,
                TokenUIAccess,
                &mut ui_access as *mut u32 as *mut std::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
                &mut returned,
            )
        };

        unsafe {
            CloseHandle(token);
        }

        queried != 0 && ui_access != 0
    }

    fn invoke_send_sas(as_user: bool) -> Result<(), String> {
        // Safety: null-terminated wide string, and the module handle is freed
        // on every return path below.
        let module = unsafe { LoadLibraryW(wide_null("sas.dll").as_ptr()) };
        if module == 0 {
            return Err("this Windows installation does not provide sas.dll".to_string());
        }

        // Safety: `b"SendSAS\0"` is a valid null-terminated ANSI name.
        let entry = unsafe { GetProcAddress(module, b"SendSAS\0".as_ptr()) };

        let Some(entry) = entry else {
            unsafe {
                FreeLibrary(module);
            }
            return Err("sas.dll on this computer does not export SendSAS".to_string());
        };

        // Safety: `SendSAS` matches `SendSasFn`, and the module stays loaded
        // for the duration of the call.
        let send_sas: SendSasFn = unsafe { std::mem::transmute(entry) };
        unsafe {
            send_sas(i32::from(as_user));
            FreeLibrary(module);
        }

        Ok(())
    }

    fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
        value
            .as_ref()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_desktop_app_is_never_eligible_whatever_the_policy_says() {
        // This is WakeMATE's shipping configuration: no uiAccess manifest, so
        // even a fully permissive policy must not let us claim success.
        for policy in [
            SAS_POLICY_NONE,
            SAS_POLICY_SERVICES,
            SAS_POLICY_EASE_OF_ACCESS,
            SAS_POLICY_BOTH,
        ] {
            assert_eq!(
                evaluate_eligibility(policy, false, false),
                SasEligibility::CallerNotPrivileged,
                "policy {policy} must not make an unprivileged caller eligible"
            );
        }
    }

    #[test]
    fn a_uiaccess_app_needs_the_ease_of_access_policy() {
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_EASE_OF_ACCESS, true, false),
            SasEligibility::Allowed
        );
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_BOTH, true, false),
            SasEligibility::Allowed
        );
        // Policy 1 covers services only, so a uiAccess app stays refused.
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_SERVICES, true, false),
            SasEligibility::PolicyDisabled
        );
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_NONE, true, false),
            SasEligibility::PolicyDisabled
        );
    }

    #[test]
    fn a_service_needs_the_services_policy() {
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_SERVICES, false, true),
            SasEligibility::Allowed
        );
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_BOTH, false, true),
            SasEligibility::Allowed
        );
        // Policy 2 covers Ease of Access apps only.
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_EASE_OF_ACCESS, false, true),
            SasEligibility::PolicyDisabled
        );
        assert_eq!(
            evaluate_eligibility(SAS_POLICY_NONE, false, true),
            SasEligibility::PolicyDisabled
        );
    }

    #[test]
    fn refusal_details_explain_what_to_change_without_leaking_host_details() {
        let not_privileged = eligibility_detail(SasEligibility::CallerNotPrivileged);
        assert!(not_privileged.contains("Program Files"));

        let policy_off = eligibility_detail(SasEligibility::PolicyDisabled);
        assert!(policy_off.contains("Secure Attention Sequence"));

        for detail in [not_privileged, policy_off] {
            assert!(
                !detail.contains('\\'),
                "details must stay path-free: {detail}"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_hosts_report_unsupported_rather_than_failing() {
        assert!(matches!(
            request_secure_attention_sequence(),
            SasOutcome::Unsupported { .. }
        ));
    }
}
