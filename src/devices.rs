//! Per-device pairing registry (Stage 3 of the migration plan).
//!
//! Each enrolled phone gets its own token so one phone can be revoked from
//! the tray without un-pairing every other phone. Only the SHA-256 hash of
//! each device token is stored on disk -- the plaintext token exists solely
//! on the phone that enrolled, so this file never needs to live in the OS
//! credential store the way the shared QR token does.

use std::{
    fs, io,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    /// Hex SHA-256 of the device token; never the token itself.
    pub token_sha256: String,
    pub approved_at_unix: u64,
}

#[derive(Debug, Default)]
pub struct DeviceRegistry {
    devices: Vec<PairedDevice>,
    /// `None` in tests that never touch disk.
    path: Option<PathBuf>,
}

impl DeviceRegistry {
    pub fn load_or_default() -> Self {
        match AppConfig::data_dir() {
            Ok(dir) => Self::load_from_path(dir.join(DEVICES_FILE)),
            Err(error) => {
                warn!(%error, "could not resolve the WakeMATE data dir; paired devices will not persist");
                Self::default()
            }
        }
    }

    pub fn load_from_path(path: PathBuf) -> Self {
        let devices = match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Vec<PairedDevice>>(&raw) {
                Ok(devices) => devices,
                Err(error) => {
                    warn!(
                        %error,
                        path = %path.display(),
                        "WakeMATE paired-device file is corrupt; starting with no paired devices"
                    );
                    Vec::new()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                warn!(
                    %error,
                    path = %path.display(),
                    "WakeMATE paired-device file is unreadable; starting with no paired devices"
                );
                Vec::new()
            }
        };

        Self {
            devices,
            path: Some(path),
        }
    }

    pub fn list(&self) -> Vec<PairedDevice> {
        self.devices.clone()
    }

    pub fn contains(&self, device_id: &str) -> bool {
        self.devices.iter().any(|device| device.id == device_id)
    }

    /// Registers a device under a caller-chosen id (the id was already handed
    /// to the phone when enrollment started). Replaces any existing record
    /// with the same id.
    pub fn register_with_id(&mut self, device_id: &str, name: &str, token: &str) -> PairedDevice {
        let device = PairedDevice {
            id: device_id.to_string(),
            name: name.to_string(),
            token_sha256: hash_token(token),
            approved_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
        };

        self.devices.retain(|existing| existing.id != device.id);
        self.devices.push(device.clone());
        self.save();
        device
    }

    /// Removes one device. Returns the removed record so the UI can name it.
    pub fn revoke(&mut self, device_id: &str) -> Option<PairedDevice> {
        let index = self
            .devices
            .iter()
            .position(|device| device.id == device_id)?;
        let removed = self.devices.remove(index);
        self.save();
        Some(removed)
    }

    /// Removes every device (token rotation and companion reset both keep
    /// their documented "revokes every phone at once" semantics).
    pub fn revoke_all(&mut self) {
        if self.devices.is_empty() {
            return;
        }
        self.devices.clear();
        self.save();
    }

    /// Returns the id of the device whose token matches, if any. The
    /// provided value is hashed first, so timing reveals nothing about the
    /// stored hashes beyond what a constant-time compare leaks (nothing).
    pub fn authenticate(&self, provided_token: &str) -> Option<String> {
        if provided_token.is_empty() {
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

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };

        if let Err(error) = self.save_to_path(path) {
            warn!(
                %error,
                path = %path.display(),
                "failed to persist the WakeMATE paired-device list"
            );
        }
    }

    fn save_to_path(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&self.devices)
            .map_err(|error| io::Error::other(error.to_string()))?;

        // Same atomic-replace pattern as the config file: a killed process
        // can leave a stray temp file but never a truncated device list.
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(format!(".{}.tmp", Uuid::new_v4()));
        let tmp_path = path.with_file_name(tmp_name);

        let write_result = (|| -> io::Result<()> {
            fs::write(&tmp_path, json.as_bytes())?;
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

    fn temp_registry() -> (DeviceRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("wakemate-devices-test-{}", Uuid::new_v4()));
        let path = dir.join(DEVICES_FILE);
        (DeviceRegistry::load_from_path(path.clone()), dir)
    }

    #[test]
    fn registered_device_authenticates_and_the_file_never_holds_the_token() {
        let (mut registry, dir) = temp_registry();

        let device = registry.register_with_id("device-1", "Test Phone", "secret-token");
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
        registry.register_with_id("device-1", "Phone A", "token-a");
        registry.register_with_id("device-2", "Phone B", "token-b");

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
        registry.register_with_id("device-1", "Phone A", "token-a");
        registry.register_with_id("device-2", "Phone B", "token-b");

        let removed = registry.revoke("device-1").unwrap();
        assert_eq!(removed.name, "Phone A");
        assert_eq!(registry.authenticate("token-a"), None);
        assert_eq!(
            registry.authenticate("token-b").as_deref(),
            Some("device-2")
        );

        registry.revoke_all();
        assert!(registry.list().is_empty());
        assert_eq!(registry.authenticate("token-b"), None);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn re_registering_the_same_id_replaces_the_old_token() {
        let (mut registry, dir) = temp_registry();
        registry.register_with_id("device-1", "Phone A", "old-token");
        registry.register_with_id("device-1", "Phone A", "new-token");

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
}
