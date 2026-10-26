//! Per-device pairing registry (Stage 3 of the migration plan).
//!
//! Each enrolled phone gets its own token so one phone can be revoked from
//! the tray without un-pairing every other phone. Only the SHA-256 hash of
//! each device token is stored on disk -- the plaintext token exists solely
//! on the phone that enrolled, so this file never needs to live in the OS
//! credential store the way the shared QR token does.

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::warn;
use uuid::Uuid;

use crate::config::AppConfig;

const DEVICES_FILE: &str = "wakemate.devices.json";

pub type SharedDeviceRegistry = Arc<Mutex<DeviceRegistry>>;

/// Status/wake are the least-privilege enrollment preset. Desktop input
/// grants broad control of the signed-in account, regardless of power flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub status: bool,
    pub wake: bool,
    pub media: bool,
    pub input: bool,
    pub power: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            status: true,
            wake: true,
            media: false,
            input: false,
            power: false,
        }
    }
}

impl Permissions {
    pub fn preset(index: i32) -> Self {
        Self {
            media: index >= 2,
            input: index >= 3,
            power: index >= 4,
            ..Self::default()
        }
    }

    pub fn label(self) -> &'static str {
        if self.input && self.power {
            "Desktop and power"
        } else if self.input {
            "Desktop control"
        } else if self.media {
            "Media remote"
        } else {
            "Status and wake"
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Capability {
    Status,
    Wake,
    Media,
    Input,
    Power,
}

impl Permissions {
    pub fn allows(self, capability: Capability) -> bool {
        match capability {
            Capability::Status => self.status,
            Capability::Wake => self.wake,
            Capability::Media => self.media,
            Capability::Input => self.input,
            Capability::Power => self.power,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryFile {
    version: u32,
    generation: u64,
    paused: bool,
    devices: Vec<PairedDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    /// Hex SHA-256 of the device token; never the token itself.
    pub token_sha256: String,
    pub approved_at_unix: u64,
    #[serde(default)]
    pub permissions: Permissions,
}

#[derive(Debug, Default)]
pub struct DeviceRegistry {
    devices: Vec<PairedDevice>,
    generation: u64,
    paused: bool,
    storage_error: Option<String>,
    /// `None` in tests that never touch disk.
    path: Option<PathBuf>,
}

impl DeviceRegistry {
    pub fn load_or_default() -> Self {
        match AppConfig::data_dir() {
            Ok(dir) => Self::load_from_path(dir.join(DEVICES_FILE)),
            Err(error) => {
                warn!(%error, "could not resolve the WakeMATE data dir; paired devices will not persist");
                Self {
                    paused: true,
                    storage_error: Some(error.to_string()),
                    ..Self::default()
                }
            }
        }
    }

    pub fn load_from_path(path: PathBuf) -> Self {
        let result = fs::read_to_string(&path).and_then(|raw| {
            if raw.trim_start().starts_with('[') {
                // Legacy approvals had no separate grants. Preserve identity,
                // but require desktop consent to restore input/power access.
                let devices =
                    serde_json::from_str::<Vec<PairedDevice>>(&raw).map_err(io::Error::other)?;
                Ok(RegistryFile {
                    version: 1,
                    generation: 0,
                    paused: false,
                    devices,
                })
            } else {
                let state: RegistryFile = serde_json::from_str(&raw).map_err(io::Error::other)?;
                if state.version != 1 {
                    return Err(io::Error::other("Unsupported device registry version"));
                }
                Ok(state)
            }
        });
        match result {
            Ok(state) => Self {
                devices: state.devices,
                generation: state.generation,
                paused: state.paused,
                storage_error: None,
                path: Some(path),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self {
                path: Some(path),
                ..Self::default()
            },
            Err(error) => {
                warn!(%error, "Device registry unavailable; all remote access is paused");
                // Preserve the original bytes. Reset is the explicit recovery path.
                if path.is_file() {
                    let _ = fs::copy(
                        &path,
                        path.with_extension(format!("{}.corrupt", Uuid::new_v4())),
                    );
                }
                Self {
                    paused: true,
                    storage_error: Some(error.to_string()),
                    path: Some(path),
                    ..Self::default()
                }
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.paused || self.storage_error.is_some()
    }

    pub fn storage_error(&self) -> Option<&str> {
        self.storage_error.as_deref()
    }

    pub fn allows(&self, id: &str, capability: Capability) -> bool {
        !self.paused()
            && self
                .devices
                .iter()
                .any(|d| d.id == id && d.permissions.allows(capability))
    }

    pub fn permissions(&self, id: &str) -> Option<Permissions> {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.permissions)
    }

    pub fn set_paused(&mut self, paused: bool) -> io::Result<()> {
        if !paused && self.storage_error.is_some() {
            return Err(io::Error::other(
                "Repair storage and reset companion before resuming",
            ));
        }
        self.paused = paused;
        self.persist_security_change()
    }

    pub fn set_permissions(&mut self, id: &str, permissions: Permissions) -> io::Result<()> {
        self.require_writable()?;
        let device = self
            .devices
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| io::Error::other("Device is no longer paired"))?;
        device.permissions = permissions;
        self.persist_security_change()
    }

    fn require_writable(&self) -> io::Result<()> {
        match &self.storage_error {
            Some(error) => Err(io::Error::other(format!(
                "Device storage requires recovery: {error}"
            ))),
            None => Ok(()),
        }
    }

    fn persist_security_change(&mut self) -> io::Result<()> {
        self.generation = self.generation.saturating_add(1);
        if let Err(error) = self.save() {
            self.paused = true;
            self.storage_error = Some(error.to_string());
            return Err(error);
        }
        Ok(())
    }

    pub fn reset(&mut self) -> io::Result<()> {
        self.paused = true;
        self.devices.clear();
        self.storage_error = None;
        self.persist_security_change()?;
        self.paused = false;
        self.persist_security_change()
    }

    pub fn list(&self) -> Vec<PairedDevice> {
        self.devices.clone()
    }

    #[cfg(test)]
    pub fn contains(&self, device_id: &str) -> bool {
        self.devices.iter().any(|device| device.id == device_id)
    }

    /// Registers a device under a caller-chosen id (the id was already handed
    /// to the phone when enrollment started). Replaces any existing record
    /// with the same id.
    #[cfg(test)]
    pub fn register_with_id(
        &mut self,
        device_id: &str,
        name: &str,
        token: &str,
    ) -> io::Result<PairedDevice> {
        self.register_with_permissions(device_id, name, token, Permissions::default())
    }

    pub fn register_with_permissions(
        &mut self,
        device_id: &str,
        name: &str,
        token: &str,
        permissions: Permissions,
    ) -> io::Result<PairedDevice> {
        self.require_writable()?;
        if self.paused() {
            return Err(io::Error::other(
                "Remote access is paused; pairing was cancelled",
            ));
        }
        let device = PairedDevice {
            id: device_id.to_string(),
            name: name.to_string(),
            token_sha256: hash_token(token),
            approved_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            permissions,
        };

        let previous = self.devices.clone();
        self.devices.retain(|existing| existing.id != device.id);
        self.devices.push(device.clone());
        if let Err(error) = self.save() {
            self.devices = previous;
            self.paused = true;
            self.storage_error = Some(error.to_string());
            return Err(error);
        }
        Ok(device)
    }

    /// Removes one device. Returns the removed record so the UI can name it.
    pub fn revoke(&mut self, device_id: &str) -> io::Result<Option<PairedDevice>> {
        let removed = self
            .devices
            .iter()
            .position(|device| device.id == device_id)
            .map(|index| self.devices.remove(index));
        // Fail closed in this process, but never tell the UI that revocation
        // survived restart unless the disk write succeeded. Retry even when
        // the in-memory record was already removed by an earlier failed save.
        self.persist_security_change()?;
        Ok(removed)
    }

    /// Removes every device (token rotation and companion reset both keep
    /// their documented "revokes every phone at once" semantics).
    pub fn revoke_all(&mut self) -> io::Result<()> {
        self.devices.clear();
        self.persist_security_change()
    }

    /// Returns the id of the device whose token matches, if any. The
    /// provided value is hashed first, so timing reveals nothing about the
    /// stored hashes beyond what a constant-time compare leaks (nothing).
    pub fn authenticate(&self, provided_token: &str) -> Option<String> {
        if provided_token.is_empty() || self.paused() {
            return None;
        }

        let provided_hash = hash_token(provided_token);
        let provided_bytes = provided_hash.as_bytes();

        self.devices
            .iter()
            .find(|device| {
                let stored = device.token_sha256.as_bytes();
                stored.len() == provided_bytes.len() && bool::from(stored.ct_eq(provided_bytes))
            })
            .map(|device| device.id.clone())
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        self.save_to_path(path)
    }

    fn save_to_path(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&RegistryFile {
            version: 1,
            generation: self.generation,
            paused: self.paused,
            devices: self.devices.clone(),
        })
        .map_err(|error| io::Error::other(error.to_string()))?;

        // Same atomic-replace pattern as the config file: a killed process
        // can leave a stray temp file but never a truncated device list.
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(format!(".{}.tmp", Uuid::new_v4()));
        let tmp_path = path.with_file_name(tmp_name);

        let write_result = (|| -> io::Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp_path, path)
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        write_result
    }
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_pause_and_legacy_identity_migrate_without_inheriting_control() {
        let (registry, dir) = temp_registry();
        let path = registry.path.clone().unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            serde_json::json!([{"id":"old", "name":"Existing Phone",
            "token_sha256":hash_token("old-token"), "approved_at_unix":1}])
            .to_string(),
        )
        .unwrap();
        let mut registry = DeviceRegistry::load_from_path(path.clone());
        assert_eq!(registry.authenticate("old-token").as_deref(), Some("old"));
        assert!(registry.allows("old", Capability::Wake));
        assert!(!registry.allows("old", Capability::Input));
        assert!(!registry.allows("old", Capability::Power));
        registry
            .set_permissions("old", Permissions::preset(3))
            .unwrap();
        assert!(registry.allows("old", Capability::Input));
        assert!(!registry.allows("old", Capability::Power));
        registry.set_paused(true).unwrap();
        let mut reloaded = DeviceRegistry::load_from_path(path);
        assert!(reloaded.paused());
        assert_eq!(reloaded.authenticate("old-token"), None);
        reloaded.set_paused(false).unwrap();
        assert!(reloaded.allows("old", Capability::Input));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_revoke_pauses_every_phone_and_corruption_requires_explicit_recovery() {
        let (mut registry, dir) = temp_registry();
        registry
            .register_with_id("a", "Phone A", "token-a")
            .unwrap();
        registry
            .register_with_id("b", "Phone B", "token-b")
            .unwrap();
        let original = registry.path.clone().unwrap();
        let blocker = dir.join("blocker");
        fs::write(&blocker, b"not a directory").unwrap();
        registry.path = Some(blocker.join("devices.json"));
        assert!(registry.revoke("a").is_err());
        assert!(registry.paused());
        assert_eq!(registry.authenticate("token-b"), None);
        assert!(registry.set_paused(false).is_err());
        registry.path = Some(original.clone());
        registry.reset().unwrap();
        assert!(!registry.paused());
        assert!(registry.list().is_empty());
        fs::write(&original, b"{broken").unwrap();
        let mut corrupt = DeviceRegistry::load_from_path(original.clone());
        assert!(corrupt.paused());
        assert!(corrupt.storage_error().is_some());
        assert_eq!(fs::read(&original).unwrap(), b"{broken");
        assert!(fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".corrupt")));
        corrupt.reset().unwrap();
        assert!(!DeviceRegistry::load_from_path(original).paused());
        fs::remove_dir_all(dir).unwrap();
    }

    fn temp_registry() -> (DeviceRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("wakemate-devices-test-{}", Uuid::new_v4()));
        let path = dir.join(DEVICES_FILE);
        (DeviceRegistry::load_from_path(path.clone()), dir)
    }

    #[test]
    fn registered_device_authenticates_and_the_file_never_holds_the_token() {
        let (mut registry, dir) = temp_registry();

        let device = registry
            .register_with_id("device-1", "Test Phone", "secret-token")
            .unwrap();
        assert_eq!(registry.authenticate("secret-token"), Some(device.id));
        assert_eq!(registry.authenticate("wrong-token"), None);
        assert_eq!(registry.authenticate(""), None);

        let raw = std::fs::read_to_string(dir.join(DEVICES_FILE)).unwrap();
        assert!(!raw.contains("secret-token"));
        assert!(raw.contains(&hash_token("secret-token")));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn registry_round_trips_through_disk() {
        let (mut registry, dir) = temp_registry();
        registry
            .register_with_id("device-1", "Phone A", "token-a")
            .unwrap();
        registry
            .register_with_id("device-2", "Phone B", "token-b")
            .unwrap();

        let reloaded = DeviceRegistry::load_from_path(dir.join(DEVICES_FILE));
        assert_eq!(reloaded.list().len(), 2);
        assert!(reloaded.contains("device-1"));
        assert_eq!(
            reloaded.authenticate("token-b").as_deref(),
            Some("device-2")
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn revoking_one_device_leaves_the_others_paired() {
        let (mut registry, dir) = temp_registry();
        registry
            .register_with_id("device-1", "Phone A", "token-a")
            .unwrap();
        registry
            .register_with_id("device-2", "Phone B", "token-b")
            .unwrap();

        let removed = registry.revoke("device-1").unwrap().unwrap();
        assert_eq!(removed.name, "Phone A");
        assert_eq!(registry.authenticate("token-a"), None);
        assert_eq!(
            registry.authenticate("token-b").as_deref(),
            Some("device-2")
        );

        registry.revoke_all().unwrap();
        assert!(registry.list().is_empty());
        assert_eq!(registry.authenticate("token-b"), None);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn re_registering_the_same_id_replaces_the_old_token() {
        let (mut registry, dir) = temp_registry();
        registry
            .register_with_id("device-1", "Phone A", "old-token")
            .unwrap();
        registry
            .register_with_id("device-1", "Phone A", "new-token")
            .unwrap();

        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.authenticate("old-token"), None);
        assert_eq!(
            registry.authenticate("new-token").as_deref(),
            Some("device-1")
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_device_file_degrades_to_an_empty_registry() {
        let dir = std::env::temp_dir().join(format!("wakemate-devices-test-{}", Uuid::new_v4()));
        let path = dir.join(DEVICES_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{not-json").unwrap();

        let registry = DeviceRegistry::load_from_path(path);
        assert!(registry.list().is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failed_registration_never_grants_a_token_and_revocation_failure_can_be_retried() {
        let (mut registry, dir) = temp_registry();
        registry
            .register_with_id("old", "Phone", "old-token")
            .unwrap();
        let real_path = registry.path.clone();
        let blocker = dir.join("not-a-directory");
        fs::write(&blocker, b"block").unwrap();
        registry.path = Some(blocker.join("devices.json"));
        assert!(registry
            .register_with_id("new", "Phone", "new-token")
            .is_err());
        assert_eq!(registry.authenticate("new-token"), None);
        assert!(registry.revoke("old").is_err());
        assert_eq!(registry.authenticate("old-token"), None);
        // The error matters: disk still contains the old credential until retry.
        assert!(DeviceRegistry::load_from_path(real_path.clone().unwrap())
            .authenticate("old-token")
            .is_some());
        registry.path = real_path.clone();
        registry.revoke("old").unwrap();
        assert!(DeviceRegistry::load_from_path(real_path.unwrap())
            .list()
            .is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
