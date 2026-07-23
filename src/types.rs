use serde::{Deserialize, Serialize};

pub const AUTH_HEADER: &str = "x-wakemate-token";

/// Version of the mobile<->companion wire contract (command schema, QR
/// payload, discovery reply). Bump on any breaking change so the mobile app
/// can detect an incompatible companion instead of failing mid-command.
/// History: 1 = implicit pre-versioning protocol; 2 = adds `mouse_button`,
/// `/v1/pairing/status`, the JSON pairing-QR payload, and this field itself.
pub const PROTOCOL_VERSION: u32 = 2;

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
