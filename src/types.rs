use serde::{Deserialize, Serialize};

pub const AUTH_HEADER: &str = "x-wakemate-token";

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
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct PairingActivationResponse {
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
