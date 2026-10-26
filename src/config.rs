use std::{
    env, fs,
    fs::OpenOptions,
    io,
    io::Write,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

pub type SharedConfig = Arc<Mutex<AppConfig>>;

const APP_DIR_NAME: &str = "WakeMATE Companion";
const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0:7777";
const DEFAULT_TLS_PORT: u16 = 7778;
const DEFAULT_DISCOVERY_MESSAGE: &str = "wakemate:discover";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub bind_address: String,
    /// HTTPS always listens on this port. `bind_address` retains the optional
    /// credential-free legacy HTTP health port.
    pub tls_port: u16,
    /// Opt-in public health compatibility only. Authenticated routes are
    /// never available through this listener, including on upgraded installs.
    pub allow_insecure_http: bool,
    pub discovery_port: u16,
    pub discovery_message: String,
    pub device_name: String,
    pub launch_on_startup: bool,
    /// LAN operation requires explicit consent in the interactive tray. An
    /// older file without this field must complete the new onboarding flow.
    pub onboarding_completed: bool,
    /// Optional private adapter address selected by the desktop user. A stale
    /// selection causes a visible bind failure instead of widening the bind.
    pub preferred_address: Option<String>,
    pub allow_remote_connections: bool,
    pub allow_discovery: bool,
}

impl AppConfig {
    pub fn prepare_install_config_at_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let new_install = !path.try_exists()?;
        let mut config = Self::load_or_create_from_path(path)?;
        if new_install {
            config.apply_install_posture();
        }
        config.save_to_path(path)?;
        Ok(config)
    }

    /// Installation may arrange normal-user startup. It cannot provide the
    /// interactive user's consent to network exposure or device permissions.
    fn apply_install_posture(&mut self) {
        self.launch_on_startup = true;
    }

    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path()?;
        Self::load_or_create_from_path(&path)
    }

    pub fn load_or_create_from_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let (config, recovered_from_error) = match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<Self>(&raw) {
                Ok(mut config) => {
                    config.normalize()?;
                    (config, false)
                }
                Err(error) => {
                    warn!(
                        %error,
                        path = %path.display(),
                        "WakeMATE config file is corrupt; preserving it and using local-only defaults"
                    );
                    let backup = path.with_extension(format!("{}.corrupt", Uuid::new_v4()));
                    fs::copy(path, backup)?;
                    (Self::default(), true)
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Self::default(), false),
            Err(error) => {
                return Err(error.into());
            }
        };

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

    /// Restores local preferences and network consent to secure defaults.
    /// Device registry revocation is a separate durable transaction at the
    /// caller; protocol 5 has no permanent shared credential to rotate.
    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::path()?;
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Legacy shared credentials and process-wide capability flags are
        // unknown fields on load and deliberately absent from this schema.
        let json = serde_json::to_string_pretty(self)?;

        // Write and flush a uniquely named sibling before atomically replacing
        // the real config. A killed process can leave an incomplete temp file,
        // but it cannot truncate the last valid config or collide with another
        // process that is starting at the same time.
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(format!(".{}.tmp", Uuid::new_v4()));
        let tmp_path = path.with_file_name(tmp_name);

        let write_result = (|| -> io::Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut tmp_file = options.open(&tmp_path)?;
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
        if self.onboarding_completed && self.allow_remote_connections {
            self.preferred_address
                .as_ref()
                .map(|address| format!("{address}:{}", self.bind_port()))
                .unwrap_or_else(|| self.bind_address.clone())
        } else {
            format!("127.0.0.1:{}", self.bind_port())
        }
    }

    pub fn effective_tls_bind_address(&self) -> String {
        let http_address = self.effective_bind_address();
        let host = http_address
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(http_address.as_str());
        format!("{host}:{}", self.tls_port)
    }

    pub fn discovery_enabled(&self) -> bool {
        self.onboarding_completed && self.allow_remote_connections && self.allow_discovery
    }

    fn normalize(&mut self) -> io::Result<()> {
        if self.bind_address.trim().is_empty() {
            self.bind_address = DEFAULT_BIND_ADDRESS.to_string();
        }

        if self.discovery_message.trim().is_empty() {
            self.discovery_message = DEFAULT_DISCOVERY_MESSAGE.to_string();
        }

        if self.tls_port == 0 {
            self.tls_port = DEFAULT_TLS_PORT;
        }

        if self.device_name.trim().is_empty() {
            self.device_name = detect_device_name();
        }

        if let Some(address) = &self.preferred_address {
            let parsed = address.trim().parse::<Ipv4Addr>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The selected network adapter must be a private IPv4 address.",
                )
            })?;
            if !parsed.is_private() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The selected network adapter must be a private IPv4 address.",
                ));
            }
            self.preferred_address = Some(parsed.to_string());
        }
        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bind_address: DEFAULT_BIND_ADDRESS.to_string(),
            tls_port: DEFAULT_TLS_PORT,
            allow_insecure_http: false,
            discovery_port: 41234,
            discovery_message: DEFAULT_DISCOVERY_MESSAGE.to_string(),
            device_name: detect_device_name(),
            launch_on_startup: true,
            onboarding_completed: false,
            preferred_address: None,
            allow_remote_connections: false,
            allow_discovery: false,
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
    use super::{AppConfig, DEFAULT_BIND_ADDRESS};

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
        assert!(!config.discovery_enabled());

        config.onboarding_completed = true;
        assert!(config.discovery_enabled());
    }

    #[test]
    fn launch_on_startup_is_enabled_by_default() {
        let config = AppConfig::default();
        assert!(config.launch_on_startup);
    }

    #[test]
    fn loading_a_legacy_file_discards_shared_secrets_and_global_capabilities() {
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("wakemate.config.json");
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = serde_json::json!({
            "api_token": "retired-shared-secret",
            "token_storage": "file",
            "allow_input_commands": true,
            "allow_power_commands": true,
            "require_auth_for_info": false,
            "allow_remote_connections": true,
            "allow_discovery": true,
            "launch_on_startup": false,
            "device_name": "Existing PC",
            "tls_port": 7443
        });
        std::fs::write(&path, legacy.to_string()).unwrap();

        let config = AppConfig::load_or_create_from_path(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let on_disk: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for retired_field in [
            "api_token",
            "token_storage",
            "allow_input_commands",
            "allow_power_commands",
            "require_auth_for_info",
        ] {
            assert!(
                on_disk.get(retired_field).is_none(),
                "{retired_field} must be retired"
            );
        }
        assert!(!raw.contains("retired-shared-secret"));
        assert_eq!(config.device_name, "Existing PC");
        assert_eq!(config.tls_port, 7443);
        assert!(!config.launch_on_startup);
        // Retain explicit preferences but require new protocol 5 consent
        // before honoring their network exposure.
        assert!(config.allow_remote_connections && config.allow_discovery);
        assert!(!config.onboarding_completed);
        assert!(!config.allow_insecure_http);
        assert_eq!(config.effective_bind_address(), "127.0.0.1:7777");
        assert_eq!(config.effective_tls_bind_address(), "127.0.0.1:7443");
        assert!(!config.discovery_enabled());
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
        // Corruption is not consent to expose the machine to the LAN.
        assert!(!config.allow_remote_connections);
        assert!(!config.allow_discovery);
        assert!(!config.allow_insecure_http);
        assert!(config.launch_on_startup);
        assert!(!config.onboarding_completed);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&raw).is_ok());
        let backup = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "corrupt"))
            .unwrap();
        assert_eq!(std::fs::read(backup.path()).unwrap(), vec![0u8; 379]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_config_file_keeps_the_locked_down_defaults() {
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("wakemate.config.json");

        let config = AppConfig::load_or_create_from_path(&path).unwrap();

        assert!(!config.allow_remote_connections);
        assert!(!config.allow_discovery);
        assert_eq!(config.effective_bind_address(), "127.0.0.1:7777");
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
    fn reset_to_defaults_clears_network_consent_and_exposure() {
        let mut config = AppConfig {
            onboarding_completed: true,
            allow_remote_connections: true,
            allow_discovery: true,
            allow_insecure_http: true,
            ..AppConfig::default()
        };

        config.reset_to_defaults();

        assert!(!config.onboarding_completed);
        assert!(!config.allow_remote_connections);
        assert!(!config.allow_discovery);
        assert!(!config.allow_insecure_http);
        assert_eq!(config.effective_tls_bind_address(), "127.0.0.1:7778");
    }

    #[test]
    fn installation_never_grants_lan_consent_and_preserves_existing_preferences() {
        let dir =
            std::env::temp_dir().join(format!("wakemate-upgrade-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("config.json");
        let mut config = AppConfig::prepare_install_config_at_path(&path).unwrap();
        assert!(config.launch_on_startup);
        assert!(!config.allow_remote_connections);
        assert!(!config.allow_discovery);
        assert!(!config.onboarding_completed);
        assert!(!config.allow_insecure_http);
        config.onboarding_completed = true;
        config.allow_remote_connections = true;
        config.allow_discovery = true;
        config.launch_on_startup = false;
        config.allow_insecure_http = true;
        config.save_to_path(&path).unwrap();
        let upgraded = AppConfig::prepare_install_config_at_path(&path).unwrap();
        assert_eq!(upgraded, config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn omitted_legacy_http_field_is_disabled_and_explicit_opt_in_is_preserved() {
        let legacy: AppConfig = serde_json::from_str("{}").unwrap();
        let modern: AppConfig = serde_json::from_str(r#"{"allow_insecure_http":false}"#).unwrap();
        let health_only: AppConfig =
            serde_json::from_str(r#"{"allow_insecure_http":true}"#).unwrap();
        assert!(!legacy.allow_insecure_http);
        assert!(!modern.allow_insecure_http);
        assert!(health_only.allow_insecure_http);
        assert!(!AppConfig::default().allow_insecure_http);
    }

    #[test]
    fn completing_onboarding_honors_the_preserved_network_preferences() {
        let mut config: AppConfig =
            serde_json::from_str(r#"{"allow_remote_connections":true,"allow_discovery":true}"#)
                .unwrap();
        assert_eq!(config.effective_bind_address(), "127.0.0.1:7777");
        config.onboarding_completed = true;
        assert_eq!(config.effective_bind_address(), DEFAULT_BIND_ADDRESS);
        assert_eq!(config.effective_tls_bind_address(), "0.0.0.0:7778");
        assert!(config.discovery_enabled());
    }

    #[test]
    fn an_unreadable_config_is_not_treated_as_a_clean_install() {
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        // Reading a directory as a file fails; the loader must return the
        // error rather than reset settings or grant network access.
        std::fs::create_dir_all(&dir).unwrap();
        assert!(AppConfig::load_or_create_from_path(&dir).is_err());
        assert!(dir.is_dir());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_preferred_adapter_only_changes_the_lan_bind_after_consent() {
        let mut config = AppConfig {
            preferred_address: Some("10.0.0.19".into()),
            allow_remote_connections: true,
            allow_discovery: true,
            ..AppConfig::default()
        };
        assert_eq!(config.effective_bind_address(), "127.0.0.1:7777");
        assert_eq!(config.effective_tls_bind_address(), "127.0.0.1:7778");
        config.onboarding_completed = true;
        assert_eq!(config.effective_bind_address(), "10.0.0.19:7777");
        assert_eq!(config.effective_tls_bind_address(), "10.0.0.19:7778");
        assert!(config.discovery_enabled());
    }

    #[test]
    fn invalid_preferred_adapters_fail_without_widening_the_saved_bind() {
        let dir =
            std::env::temp_dir().join(format!("wakemate-config-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wakemate.config.json");
        for address in [
            "",
            "localhost",
            "0.0.0.0",
            "8.8.8.8",
            "127.0.0.1",
            "::1",
            "10.0.0.999",
        ] {
            let raw = serde_json::json!({
                "preferred_address": address,
                "onboarding_completed": true,
                "allow_remote_connections": true,
            })
            .to_string();
            std::fs::write(&path, &raw).unwrap();
            assert!(
                AppConfig::load_or_create_from_path(&path).is_err(),
                "{address}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
