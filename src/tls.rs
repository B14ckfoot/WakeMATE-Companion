use std::{
    fs,
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
};

use axum_server::tls_rustls::RustlsConfig;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::AppConfig;

const TLS_IDENTITY_FILE: &str = "wakemate.tls.json";

#[derive(Clone, Debug)]
pub struct TlsIdentity {
    certificate_pem: Vec<u8>,
    private_key_pem: Vec<u8>,
    fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredTlsIdentity {
    certificate_pem: String,
    private_key_pem: String,
}

impl TlsIdentity {
    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        Self::load_or_create_at(&Self::path()?)
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub async fn rustls_config(&self) -> Result<RustlsConfig, Box<dyn std::error::Error>> {
        install_crypto_provider();
        Ok(
            RustlsConfig::from_pem(self.certificate_pem.clone(), self.private_key_pem.clone())
                .await?,
        )
    }

    fn path() -> io::Result<PathBuf> {
        Ok(AppConfig::data_dir()?.join(TLS_IDENTITY_FILE))
    }

    fn load_or_create_at(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        match fs::read(path) {
            Ok(raw) => return Self::from_json(&raw),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let generated = Self::generate()?;
        let stored = StoredTlsIdentity {
            certificate_pem: String::from_utf8(generated.certificate_pem.clone())?,
            private_key_pem: String::from_utf8(generated.private_key_pem.clone())?,
        };
        let serialized = serde_json::to_vec_pretty(&stored)?;
        let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
        temp_name.push(format!(".{}.tmp", Uuid::new_v4()));
        let temp_path = path.with_file_name(temp_name);

        let write_result = (|| -> io::Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }

            let mut file = options.open(&temp_path)?;
            file.write_all(&serialized)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp_path, path)
        })();

        match write_result {
            Ok(()) => Ok(generated),
            // A tray instance and the pre-logon server can start at nearly
            // the same time. If the other process won the race, use its
            // identity so both listeners advertise the same stable pin.
            Err(_) if path.exists() => {
                let _ = fs::remove_file(&temp_path);
                Self::from_json(&fs::read(path)?)
            }
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                Err(error.into())
            }
        }
    }

    fn generate() -> Result<Self, Box<dyn std::error::Error>> {
        install_crypto_provider();
        let CertifiedKey { cert, signing_key } = generate_simple_self_signed(vec![
            "localhost".to_string(),
            "wakemate.local".to_string(),
            "127.0.0.1".to_string(),
        ])?;
        let certificate_pem = cert.pem().into_bytes();
        let private_key_pem = signing_key.serialize_pem().into_bytes();
        let fingerprint = fingerprint_for_certificate(&certificate_pem)?;

        Ok(Self {
            certificate_pem,
            private_key_pem,
            fingerprint,
        })
    }

    fn from_json(raw: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let stored: StoredTlsIdentity = serde_json::from_slice(raw)?;
        if stored.private_key_pem.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stored TLS private key is empty",
            )
            .into());
        }

        let certificate_pem = stored.certificate_pem.into_bytes();
        let fingerprint = fingerprint_for_certificate(&certificate_pem)?;
        Ok(Self {
            certificate_pem,
            private_key_pem: stored.private_key_pem.into_bytes(),
            fingerprint,
        })
    }
}

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn fingerprint_for_certificate(
    certificate_pem: &[u8],
) -> Result<String, Box<dyn std::error::Error>> {
    let block = pem::parse(certificate_pem)?;
    if block.tag() != "CERTIFICATE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TLS identity does not contain an X.509 certificate",
        )
        .into());
    }

    let digest = Sha256::digest(block.contents());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identity_is_reused_with_a_stable_sha256_fingerprint() {
        let dir = std::env::temp_dir().join(format!("wakemate-tls-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TLS_IDENTITY_FILE);

        let first = TlsIdentity::load_or_create_at(&path).unwrap();
        let second = TlsIdentity::load_or_create_at(&path).unwrap();

        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.fingerprint().len(), 64);
        assert!(first
            .fingerprint()
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert!(path.exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_identity_is_rejected_instead_of_silently_changing_the_pin() {
        let dir = std::env::temp_dir().join(format!("wakemate-tls-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TLS_IDENTITY_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{not-json").unwrap();

        assert!(TlsIdentity::load_or_create_at(&path).is_err());

        std::fs::remove_dir_all(dir).ok();
    }
}
