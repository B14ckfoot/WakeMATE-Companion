//! Wraps the OS-native secret store (Windows Credential Manager, macOS/iOS
//! Keychain, or the platform `keyring` backend on Linux) for the one secret
//! WakeMATE needs to protect: the pairing token.
//!
//! Test builds use an in-memory fake (see the `backend` module below)
//! instead of the real OS credential store so that `cargo test` never
//! creates or rotates a real Windows Credential Manager / Keychain entry on
//! the machine running the tests.

pub use backend::{delete_token, load_token, store_token};

#[cfg(not(test))]
mod backend {
    use keyring::Entry;
    use tracing::warn;

    const SERVICE_NAME: &str = "WakeMATE Companion";
    const ACCOUNT_NAME: &str = "pairing-token";

    fn entry() -> Result<Entry, String> {
        Entry::new(SERVICE_NAME, ACCOUNT_NAME).map_err(|error| error.to_string())
    }

    /// Reads the pairing token from the OS credential store. Returns `None`
    /// if there is no stored entry yet, or if the platform backend is
    /// unavailable -- callers fall back to file-based storage in that case
    /// rather than failing to start.
    pub fn load_token() -> Option<String> {
        let entry = entry()
            .inspect_err(|error| warn!(%error, "could not open the OS credential store"))
            .ok()?;

        match entry.get_password() {
            Ok(token) if !token.trim().is_empty() => Some(token),
            Ok(_) => None,
            Err(keyring::Error::NoEntry) => None,
            Err(error) => {
                warn!(%error, "failed to read the pairing token from the OS credential store");
                None
            }
        }
    }

    /// Stores the pairing token in the OS credential store.
    pub fn store_token(token: &str) -> Result<(), String> {
        let entry = entry()?;
        entry.set_password(token).map_err(|error| error.to_string())
    }

    /// Removes the stored pairing token, if any. A missing entry is not
    /// treated as an error.
    pub fn delete_token() {
        match entry() {
            Ok(entry) => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(error) => {
                    warn!(%error, "failed to delete the pairing token from the OS credential store")
                }
            },
            Err(error) => {
                warn!(%error, "could not open the OS credential store to delete the pairing token")
            }
        }
    }
}

#[cfg(test)]
mod backend {
    use std::cell::RefCell;

    thread_local! {
        static FAKE_STORE: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    pub fn load_token() -> Option<String> {
        FAKE_STORE.with(|store| store.borrow().clone())
    }

    pub fn store_token(token: &str) -> Result<(), String> {
        FAKE_STORE.with(|store| *store.borrow_mut() = Some(token.to_string()));
        Ok(())
    }

    pub fn delete_token() {
        FAKE_STORE.with(|store| *store.borrow_mut() = None);
    }
}
