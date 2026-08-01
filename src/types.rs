use serde::{Deserialize, Serialize};

pub const AUTH_HEADER: &str = "x-wakemate-token";

/// Version of the mobile<->companion wire contract (command schema, QR
/// payload, discovery reply). Bump on any breaking change so the mobile app
/// can detect an incompatible companion instead of failing mid-command.
/// History: 1 = implicit pre-versioning protocol; 2 = adds `mouse_button`,
/// `/v1/pairing/status`, the JSON pairing-QR payload, and this field itself;
/// 3 = adds `/v1/pairing/enroll` per-device tokens and the `device_id` query
/// on `/v1/pairing/status`; 4 = adds the `security_screen` command and stops
/// accepting `ctrl+alt+delete` as a `key_press`.
pub const PROTOCOL_VERSION: u32 = 4;

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(message: impl Into<String>, data: T) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Success with no payload, for endpoints whose response type is generic
    /// because *some* of their outcomes carry data. `data` is skipped during
    /// serialization, so the wire format is unchanged for the others.
    pub fn ok_empty(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: None,
        }
    }

    /// A completed request whose outcome was not success -- used where the
    /// payload itself explains the failure and an HTTP error status would
    /// throw away that detail.
    pub fn failed(message: impl Into<String>, data: T) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: Some(data),
        }
    }
}

impl ApiResponse<()> {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub device_name: String,
    pub version: String,
    pub protocol_version: u32,
}

#[derive(Debug, Serialize)]
pub struct InfoResponse {
    pub device_name: String,
    pub local_ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnet_mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_address: Option<String>,
    pub wol_port: u16,
    pub bind_address: String,
    pub discovery_port: u16,
    pub allow_remote_connections: bool,
    pub allow_discovery: bool,
    pub allow_input_commands: bool,
    pub allow_power_commands: bool,
}

#[derive(Debug, Serialize)]
pub struct DiscoveryResponse {
    pub device_name: String,
    pub local_ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    pub api_port: u16,
    pub tls_port: u16,
    /// Lowercase SHA-256 digest of the leaf certificate's DER bytes. The
    /// mobile app only trusts this value after the same digest arrives over
    /// the visual QR pairing channel.
    pub fp: String,
    pub version: String,
    pub protocol_version: u32,
}

#[derive(Debug, Serialize)]
pub struct PairingActivationResponse {
    pub allow_input_commands: bool,
    pub allow_power_commands: bool,
    /// `"pending_approval"` until the person at the desktop approves the
    /// prompt; poll `/v1/pairing/check` or `/v1/info` afterwards to see the
    /// flags flip to `true`.
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PairingStatusResponse {
    /// `"idle"` (no activation requested this session), `"pending"` (a
    /// desktop prompt is showing), `"approved"`, or `"denied"`. Denials are
    /// per-prompt, not permanent: a later activation shows a fresh prompt.
    pub approval: &'static str,
    pub allow_input_commands: bool,
    pub allow_power_commands: bool,
}

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    /// Friendly name the phone reports for itself, shown in the desktop
    /// approval prompt and the tray's paired-device list.
    pub device_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub device_id: String,
    /// The per-device secret. Returned exactly once, over the (ideally
    /// pinned-TLS) enrollment response; the companion stores only its hash,
    /// and it authenticates nothing until the desktop approves.
    pub device_token: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct PairingStatusQuery {
    /// When set, report the approval state of this specific enrollment
    /// instead of the process-wide legacy state.
    pub device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WakeRequest {
    pub mac: String,
    pub broadcast: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandRequest {
    Wake {
        mac: String,
        broadcast: Option<String>,
        port: Option<u16>,
    },
    MouseMove {
        delta_x: i32,
        delta_y: i32,
    },
    MouseClick {
        button: Option<MouseButtonArg>,
        double: Option<bool>,
    },
    /// Press or release a button without the paired click, so the phone can
    /// drive drag/hold gestures. Matches what the mobile control screen has
    /// always sent (`{"type":"mouse_button","button":"left","action":"down"}`).
    MouseButton {
        button: Option<MouseButtonArg>,
        action: MouseButtonAction,
    },
    MouseScroll {
        direction: ScrollDirection,
        amount: Option<i32>,
    },
    KeyPress {
        key: String,
    },
    TextInput {
        text: String,
    },
    Media {
        action: MediaAction,
    },
    System {
        action: SystemAction,
    },
    /// Ask Windows to show the Ctrl+Alt+Delete security screen.
    ///
    /// This is deliberately *not* a `key_press`: Windows filters synthetic
    /// Ctrl+Alt+Delete out of the Secure Attention Sequence path, so sending
    /// it as keystrokes silently does nothing useful (see
    /// `crate::secure_attention`). Unlike every other command, this one
    /// answers `200 OK` with a [`SecurityCommandResult`] describing what
    /// actually happened, so the phone never has to infer success from a
    /// status code.
    SecurityScreen {
        /// What to do when Windows refuses the real sequence. Defaults to
        /// [`SecurityFallback::None`], which changes nothing on the desktop
        /// and reports `permission_required`, so a caller only ever gets the
        /// substitute action it explicitly asked for.
        #[serde(default)]
        fallback: SecurityFallback,
    },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFallback {
    #[default]
    None,
    /// Lock the workstation -- the one thing a remote user reliably wants
    /// Ctrl+Alt+Delete for that a desktop app is actually allowed to do.
    Lock,
}

/// Outcome vocabulary shared with the mobile app. `unauthorized` and
/// `offline` are not produced here: a bad token is a `401` from
/// [`crate::app`], and an unreachable companion never answers at all, so the
/// phone derives those two from the transport.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCommandStatus {
    /// The requested action, or the caller's chosen fallback, was carried out.
    Success,
    /// This companion's OS has no equivalent action.
    Unsupported,
    /// The action exists but Windows will not let this companion perform it.
    PermissionRequired,
    /// Eligible and attempted, but it failed.
    ExecutionFailed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCommandAction {
    /// Windows raised the real Ctrl+Alt+Delete security screen.
    SecureAttentionSequence,
    /// The workstation was locked instead.
    Lock,
    /// Nothing was done to the desktop.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityCommandResult {
    pub status: SecurityCommandStatus,
    /// What actually happened, which is not always what was requested.
    pub action: SecurityCommandAction,
    /// True when `action` is the caller's fallback rather than the real
    /// sequence, so the phone can tell the user the plain truth.
    pub fallback_used: bool,
    /// Operator-facing explanation. Safe to surface: it never contains
    /// tokens, paths, or peer addresses.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButtonArg {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButtonAction {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAction {
    PlayPause,
    Next,
    Previous,
    Mute,
    VolumeUp,
    VolumeDown,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemAction {
    Sleep,
    Restart,
    Shutdown,
    Lock,
    Logoff,
}

#[cfg(test)]
mod tests {
    use super::*;

    // These fixtures are the exact bytes the mobile app sends (see
    // Wakemate-Mobile/src/services/deviceService.ts, mapLegacyCommand).
    // If one stops parsing, the mobile integration is broken.

    #[test]
    fn mouse_button_down_matches_the_mobile_wire_format() {
        let parsed: CommandRequest =
            serde_json::from_str(r#"{"type":"mouse_button","button":"left","action":"down"}"#)
                .unwrap();
        assert!(matches!(
            parsed,
            CommandRequest::MouseButton {
                button: Some(MouseButtonArg::Left),
                action: MouseButtonAction::Down,
            }
        ));
    }

    #[test]
    fn mouse_button_up_matches_the_mobile_wire_format() {
        let parsed: CommandRequest =
            serde_json::from_str(r#"{"type":"mouse_button","button":"right","action":"up"}"#)
                .unwrap();
        assert!(matches!(
            parsed,
            CommandRequest::MouseButton {
                button: Some(MouseButtonArg::Right),
                action: MouseButtonAction::Up,
            }
        ));
    }

    #[test]
    fn mouse_move_matches_the_mobile_wire_format() {
        let parsed: CommandRequest =
            serde_json::from_str(r#"{"type":"mouse_move","delta_x":12,"delta_y":-8}"#).unwrap();
        assert!(matches!(
            parsed,
            CommandRequest::MouseMove {
                delta_x: 12,
                delta_y: -8,
            }
        ));
    }

    #[test]
    fn security_screen_matches_the_mobile_wire_format() {
        let parsed: CommandRequest =
            serde_json::from_str(r#"{"type":"security_screen","fallback":"lock"}"#).unwrap();
        assert!(matches!(
            parsed,
            CommandRequest::SecurityScreen {
                fallback: SecurityFallback::Lock,
            }
        ));
    }

    #[test]
    fn security_screen_without_a_fallback_defaults_to_touching_nothing() {
        let parsed: CommandRequest = serde_json::from_str(r#"{"type":"security_screen"}"#).unwrap();
        assert!(matches!(
            parsed,
            CommandRequest::SecurityScreen {
                fallback: SecurityFallback::None,
            }
        ));
    }

    #[test]
    fn unknown_command_types_are_rejected_rather_than_guessed_at() {
        // The tagged enum is the command allowlist; anything outside it must
        // fail to parse instead of reaching an execution path.
        for payload in [
            r#"{"type":"ctrl_alt_delete"}"#,
            r#"{"type":"exec","command":"calc.exe"}"#,
            r#"{"type":"security_screen","fallback":"shutdown"}"#,
        ] {
            assert!(
                serde_json::from_str::<CommandRequest>(payload).is_err(),
                "{payload} must not parse"
            );
        }
    }

    #[test]
    fn security_result_serializes_the_statuses_the_mobile_app_switches_on() {
        let json = serde_json::to_value(SecurityCommandResult {
            status: SecurityCommandStatus::PermissionRequired,
            action: SecurityCommandAction::None,
            fallback_used: false,
            detail: "policy is off".to_string(),
        })
        .unwrap();

        assert_eq!(json["status"], "permission_required");
        assert_eq!(json["action"], "none");
        assert_eq!(json["fallback_used"], false);
    }

    #[test]
    fn a_failed_response_still_carries_its_payload() {
        // The phone needs `status` even when the command did not succeed, so
        // this response must not collapse to a bare error message.
        let response = ApiResponse::failed(
            "security screen unavailable",
            SecurityCommandResult {
                status: SecurityCommandStatus::Unsupported,
                action: SecurityCommandAction::None,
                fallback_used: false,
                detail: "not Windows".to_string(),
            },
        );
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["ok"], false);
        assert_eq!(json["data"]["status"], "unsupported");
    }

    #[test]
    fn responses_without_data_do_not_emit_a_data_field() {
        // Every other command shares this response type now; their wire
        // format must be byte-identical to before.
        let response = ApiResponse::<SecurityCommandResult>::ok_empty("mouse click sent");
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["ok"], true);
        assert_eq!(json["message"], "mouse click sent");
        assert!(json.get("data").is_none());
    }

    #[test]
    fn health_response_includes_the_protocol_version() {
        let response = HealthResponse {
            status: "online",
            device_name: "Desk Rig".to_string(),
            version: "0.1.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
    }
}
