use std::{
    env, fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type SharedConfig = Arc<Mutex<AppConfig>>;

const APP_DIR_NAME: &str = "WakeMATE Companion";
const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0:7777";
const DEFAULT_DISCOVERY_MESSAGE: &str = "wakemate:discover";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub bind_address: String,
    pub discovery_port: u16,
    pub discovery_message: String,
    pub api_token: String,
    pub device_name: String,
    pub launch_on_startup: bool,
    pub allow_input_commands: bool,
    pub allow_power_commands: bool,
    pub allow_remote_connections: bool,
    pub allow_discovery: bool,
    pub require_auth_for_info: bool,
}

impl AppConfig {
    pub fn prepare_install_config() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path()?;
        Self::prepare_install_config_at_path(&path)
    }

    pub fn prepare_install_config_at_path(
        path: &PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = Self::load_or_create_from_path(path)?;
        config.launch_on_startup = true;
        config.allow_remote_connections = true;
        config.allow_discovery = true;
        config.save_to_path(path)?;
        Ok(config)
    }

    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path()?;
        Self::load_or_create_from_path(&path)
    }

    pub fn load_or_create_from_path(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.clone();

        if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let mut config: Self = serde_json::from_str(&raw)?;
            config.normalize();
            return Ok(config);
        }

        let config = Self::default();
        config.save_to_path(&path)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::path()?;
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let path = path.clone();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn path() -> io::Result<PathBuf> {
        Ok(Self::data_dir()?.join("wakemate.config.json"))
    }

    pub fn data_dir() -> io::Result<PathBuf> {
        let path = app_data_root()?.join(APP_DIR_NAME);
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    pub fn executable_dir() -> io::Result<PathBuf> {
        Ok(std::env::current_exe()?
            .parent()
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?))
    }

    pub fn asset_dir() -> io::Result<PathBuf> {
        let cwd_assets = std::env::current_dir()?.join("assets");
        if cwd_assets.exists() {
            return Ok(cwd_assets);
        }

        Ok(Self::executable_dir()?.join("assets"))
    }

    pub fn bind_port(&self) -> u16 {
        self.bind_address
            .rsplit(':')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(7777)
    }

    pub fn effective_bind_address(&self) -> String {
        if self.allow_remote_connections {
            self.bind_address.clone()
        } else {
            format!("127.0.0.1:{}", self.bind_port())
        }
    }

    pub fn discovery_enabled(&self) -> bool {
        self.allow_remote_connections && self.allow_discovery
    }

    pub fn rotate_api_token(&mut self) -> String {
        self.api_token = Uuid::new_v4().to_string();
        self.api_token.clone()
    }

    fn normalize(&mut self) {
        if self.bind_address.trim().is_empty() {
            self.bind_address = DEFAULT_BIND_ADDRESS.to_string();
        }

        if self.discovery_message.trim().is_empty() {
            self.discovery_message = DEFAULT_DISCOVERY_MESSAGE.to_string();
        }

        if self.api_token.trim().is_empty() {
            self.api_token = Uuid::new_v4().to_string();
        }

        if self.device_name.trim().is_empty() {
            self.device_name = detect_device_name();
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bind_address: DEFAULT_BIND_ADDRESS.to_string(),
            discovery_port: 41234,
            discovery_message: DEFAULT_DISCOVERY_MESSAGE.to_string(),
            api_token: Uuid::new_v4().to_string(),
            device_name: detect_device_name(),
            launch_on_startup: true,
            allow_input_commands: false,
            allow_power_commands: false,
            allow_remote_connections: false,
            allow_discovery: false,
            require_auth_for_info: true,
        }
    }
}

fn detect_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "WakeMATE Companion".to_string())
}

fn app_data_root() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("APPDATA is not set"))
    }

    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library").join("Application Support"))
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
            return Ok(path);
        }

        home_dir().map(|home| home.join(".config"))
    }
}

#[cfg(not(target_os = "windows"))]
fn home_dir() -> io::Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| io::Error::other("HOME is not set"))
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn localhost_is_the_secure_default_bind() {
        let config = AppConfig::default();
        assert_eq!(config.effective_bind_address(), "127.0.0.1:7777");
    }

    #[test]
    fn discovery_requires_remote_connections() {
        let mut config = AppConfig::default();
        config.allow_discovery = true;
        assert!(!config.discovery_enabled());

        config.allow_remote_connections = true;
        assert!(config.discovery_enabled());
    }

    #[test]
    fn token_rotation_replaces_the_existing_secret() {
        let mut config = AppConfig::default();
        let old_token = config.api_token.clone();
        let new_token = config.rotate_api_token();

        assert_ne!(old_token, new_token);
        assert_eq!(config.api_token, new_token);
    }

    #[test]
    fn launch_on_startup_is_enabled_by_default() {
        let config = AppConfig::default();
        assert!(config.launch_on_startup);
    }
}
