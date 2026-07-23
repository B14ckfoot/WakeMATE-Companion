use std::{
    env, fs,
    fs::OpenOptions,
    io,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::credential_store;

pub type SharedConfig = Arc<Mutex<AppConfig>>;

const APP_DIR_NAME: &str = "WakeMATE Companion";
const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0:7777";
const DEFAULT_DISCOVERY_MESSAGE: &str = "wakemate:discover";

/// Records where the pairing token currently lives so the UI can be honest
/// about it and so `save_to_path` knows whether it is safe to omit the
/// secret from the on-disk config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenStorage {
    /// Windows Credential Manager / macOS Keychain / platform keyring.
    Keyring,
    /// Fallback: kept in the plaintext config file because the OS
    /// credential store was unavailable on this machine.
    #[default]
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub bind_address: String,
    pub discovery_port: u16,
    pub discovery_message: String,
    /// In-memory pairing token. When `token_storage` is `Keyring` this is
    /// hydrated from the OS credential store at load time and blanked out
    /// before the struct is written back to disk; see `hydrate_token` and
    /// `save_to_path`.
    pub api_token: String,
    pub token_storage: TokenStorage,
    pub device_name: String,
    pub launch_on_startup: bool,
    pub allow_input_commands: bool,
    pub allow_power_commands: bool,
    pub allow_remote_connections: bool,
    pub allow_discovery: bool,
    pub require_auth_for_info: bool,
}

impl AppConfig {
    pub fn prepare_install_config_at_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Self {
            launch_on_startup: true,
            allow_remote_connections: true,
            allow_discovery: true,
            ..Self::load_or_create_from_path(path)?
        };
        config.save_to_path(path)?;
        Ok(config)
    }

    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path()?;
        Self::load_or_create_from_path(&path)
    }

    pub fn load_or_create_from_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let (mut config, recovered_from_error) = match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<Self>(&raw) {
                Ok(mut config) => {
                    config.normalize();
                    (config, false)
                }
                Err(error) => {
                    warn!(
                        %error,
                        path = %path.display(),
                        "WakeMATE config file is corrupt; resetting to defaults"
                    );
                    (Self::default(), true)
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Self::default(), false),
            Err(error) => {
                // An unreadable config must not be a fatal startup error:
                // this binary has no console to report one. Use defaults
                // even if the repair write below also fails.
                warn!(
                    %error,
                    path = %path.display(),
                    "WakeMATE config file is unreadable; resetting to defaults"
                );
                (Self::default(), true)
            }
        };

        config.hydrate_token();
        if let Err(error) = config.save_to_path(path) {
            if recovered_from_error {
                warn!(
                    %error,
                    path = %path.display(),
                    "WakeMATE could not replace the unreadable config; continuing with in-memory defaults"
                );
            } else {
                return Err(error);
            }
        }
        Ok(config)
    }

    /// Resolves the active pairing token, preferring the OS credential store.
    ///
    /// - If the credential store already has an entry, it wins (it is the
    ///   source of truth once migration has happened).
    /// - Otherwise, whatever is already in `self.api_token` is used (a fresh
    ///   UUID for a brand new config, or a legacy plaintext value read from
    ///   an older config file) and an attempt is made to migrate it into the
    ///   credential store.
    /// - If the credential store is unavailable on this machine, the token
    ///   stays in the config file and that fact is recorded in
    ///   `token_storage` so the UI can surface it honestly.
    fn hydrate_token(&mut self) {
        if let Some(token) = credential_store::load_token() {
            self.api_token = token;
            self.token_storage = TokenStorage::Keyring;
            return;
        }

        let token = if self.api_token.trim().is_empty() {
            Uuid::new_v4().to_string()
        } else {
            self.api_token.clone()
        };

        match credential_store::store_token(&token) {
            Ok(()) => {
                self.api_token = token;
                self.token_storage = TokenStorage::Keyring;
            }
            Err(error) => {
                warn!(
                    %error,
                    "OS credential store unavailable; keeping the pairing token in the config file"
                );
                self.api_token = token;
                self.token_storage = TokenStorage::File;
            }
        }
    }

    /// Clears all local state (including the stored pairing token) and
    /// starts over with fresh secure defaults. This only touches local
    /// device state -- WakeMATE has no cloud account to sign out of.
    pub fn reset_to_defaults(&mut self) {
        credential_store::delete_token();
        *self = Self::default();
        self.hydrate_token();
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::path()?;
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Once the token lives in the OS credential store there is no
        // reason for a plaintext copy to also sit in the config file.
        let mut on_disk = self.clone();
        if on_disk.token_storage == TokenStorage::Keyring {
            on_disk.api_token = String::new();
        }

        let json = serde_json::to_string_pretty(&on_disk)?;

        // Write and flush a uniquely named sibling before atomically replacing
        // the real config. A killed process can leave an incomplete temp file,
        // but it cannot truncate the last valid config or collide with another
        // process that is starting at the same time.
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(format!(".{}.tmp", Uuid::new_v4()));
        let tmp_path = path.with_file_name(tmp_name);

        let write_result = (|| -> io::Result<()> {
            let mut tmp_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            tmp_file.write_all(json.as_bytes())?;
            tmp_file.sync_all()?;
            drop(tmp_file);
            fs::rename(&tmp_path, path)
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        write_result?;
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

    /// Rotates the pairing token, preferring to store the new value in the
    /// OS credential store. Returns the new token; callers are expected to
    /// call `save()` afterwards as with any other config change.
    pub fn rotate_api_token(&mut self) -> String {
        let new_token = Uuid::new_v4().to_string();

        match credential_store::store_token(&new_token) {
            Ok(()) => self.token_storage = TokenStorage::Keyring,
            Err(error) => {
                warn!(
                    %error,
                    "OS credential store unavailable; rotated token will be kept in the config file"
                );
                self.token_storage = TokenStorage::File;
            }
        }

        self.api_token = new_token.clone();
        new_token
    }

    fn normalize(&mut self) {
        if self.bind_address.trim().is_empty() {
            self.bind_address = DEFAULT_BIND_ADDRESS.to_string();
        }

        if self.discovery_message.trim().is_empty() {
            self.discovery_message = DEFAULT_DISCOVERY_MESSAGE.to_string();
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
            api_token: String::new(),
            token_storage: TokenStorage::default(),
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
    use super::{AppConfig, TokenStorage, DEFAULT_BIND_ADDRESS};
    use crate::credential_store;

    #[test]
    fn localhost_is_the_secure_default_bind() {
        let config = AppConfig::default();
        assert_eq!(config.effective_bind_address(), "127.0.0.1:7777");
    }

    #[test]
    // Field-by-field mutation is the point of this test: it asserts on the
    // state *between* the two flags being set, which a struct literal can't express.
    #[allow(clippy::field_reassign_with_default)]
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
        assert_eq!(config.token_storage, TokenStorage::Keyring);
    }

    #[test]
    fn launch_on_startup_is_enabled_by_default() {
        let config = AppConfig::default();
        assert!(config.launch_on_startup);
    }

    #[test]
    fn hydrate_token_migrates_a_legacy_plaintext_value_into_the_credential_store() {
        let mut config = AppConfig {
            api_token: "legacy-plaintext-token".to_string(),
            ..AppConfig::default()
        };

        config.hydrate_token();

        assert_eq!(config.api_token, "legacy-plaintext-token");
        assert_eq!(config.token_storage, TokenStorage::Keyring);
        assert_eq!(
            credential_store::load_token().as_deref(),
            Some("legacy-plaintext-token")
        );
    }

    #[test]
    fn hydrate_token_generates_a_fresh_token_when_none_exists() {
        let mut config = AppConfig::default();
        assert!(config.api_token.is_empty());

        config.hydrate_token();

        assert!(!config.api_token.is_empty());
        assert_eq!(config.token_storage, TokenStorage::Keyring);
    }

    #[test]
    fn save_to_path_blanks_the_token_once_it_is_in_the_credential_store() {
        let mut config = AppConfig::default();
        config.hydrate_token();
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("wakemate.config.json");

        config.save_to_path(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let on_disk: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(on_disk["api_token"], "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_or_create_from_path_recovers_from_a_zero_filled_config_file() {
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wakemate.config.json");
        std::fs::write(&path, vec![0u8; 379]).unwrap();

        let config = AppConfig::load_or_create_from_path(&path).unwrap();

        assert_eq!(config.bind_address, DEFAULT_BIND_ADDRESS);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&raw).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_to_path_atomically_replaces_an_existing_config() {
        let mut config = AppConfig::default();
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("wakemate.config.json");

        config.save_to_path(&path).unwrap();
        config.allow_remote_connections = true;
        config.save_to_path(&path).unwrap();

        let saved: AppConfig =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.allow_remote_connections);
        let leftover_temp_file = std::fs::read_dir(&dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("wakemate.config.json.")
        });
        assert!(!leftover_temp_file);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reset_to_defaults_replaces_the_stored_token() {
        let mut config = AppConfig::default();
        config.hydrate_token();
        let original_token = config.api_token.clone();
        config.allow_input_commands = true;

        config.reset_to_defaults();

        assert_ne!(config.api_token, original_token);
        assert!(!config.allow_input_commands);
        assert_eq!(
            credential_store::load_token().as_deref(),
            Some(config.api_token.as_str())
        );
    }
}
