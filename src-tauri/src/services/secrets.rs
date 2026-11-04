//! Secrets in the OS keychain (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux), via the `keyring` crate directly so services
//! don't need a Tauri `AppHandle`.
//!
//! Entries live under the app identifier as the keychain *service*.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Backend {
    Keychain {
        service: String,
    },
    /// For tests: never touches the real keychain
    #[cfg(test)]
    Memory(Arc<Mutex<HashMap<String, String>>>),
}

#[derive(Clone)]
pub struct Secrets(Backend);

const OPEN_FAILED: &str = "Failed to open the system keychain";
/// For tests: a stored value that reads as an error
#[cfg(test)]
const UNREADABLE: &str = "[unreadable in tests]";

impl Secrets {
    pub fn keychain(service: impl Into<String>) -> Self {
        Self(Backend::Keychain {
            service: service.into(),
        })
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self(Backend::Memory(Arc::default()))
    }

    /// The stored secret, or `None` if there is none. Like the plugin did,
    /// an unreadable entry is treated as missing (and logged).
    pub fn get(&self, user: &str) -> Result<Option<String>, String> {
        match self.read(user) {
            Err(e) if !e.starts_with(OPEN_FAILED) => {
                log::warn!("{e}");
                Ok(None)
            }
            other => other,
        }
    }

    /// Like [`Self::get`], but an entry that exists and can't be read is an
    /// error, for values where "missing" and "unreadable" mean different
    /// things (the free trial's counter)
    pub fn read(&self, user: &str) -> Result<Option<String>, String> {
        match &self.0 {
            Backend::Keychain { service } => {
                let entry = keyring::Entry::new(service, user)
                    .map_err(|e| format!("{OPEN_FAILED}: {e}"))?;
                match entry.get_password() {
                    Ok(secret) => Ok(Some(secret)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(format!("Could not read keychain entry '{user}': {e}")),
                }
            }
            #[cfg(test)]
            Backend::Memory(map) => match lock(map).get(user) {
                Some(value) if value == UNREADABLE => {
                    Err(format!("Could not read keychain entry '{user}': locked"))
                }
                value => Ok(value.cloned()),
            },
        }
    }

    /// For tests: the entry exists but can't be read, like a locked keychain
    #[cfg(test)]
    pub fn make_unreadable(&self, user: &str) {
        if let Backend::Memory(map) = &self.0 {
            lock(map).insert(user.to_string(), UNREADABLE.to_string());
        }
    }

    pub fn set(&self, user: &str, secret: &str) -> Result<(), String> {
        match &self.0 {
            Backend::Keychain { service } => keyring::Entry::new(service, user)
                .and_then(|entry| entry.set_password(secret))
                .map_err(|e| format!("Failed to save to the system keychain: {e}")),
            #[cfg(test)]
            Backend::Memory(map) => {
                lock(map).insert(user.to_string(), secret.to_string());
                Ok(())
            }
        }
    }

    /// Removes the secret; a missing entry is not an error.
    pub fn delete(&self, user: &str) -> Result<(), String> {
        match &self.0 {
            Backend::Keychain { service } => {
                match keyring::Entry::new(service, user).and_then(|e| e.delete_credential()) {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(format!("Failed to remove from the system keychain: {e}")),
                }
            }
            #[cfg(test)]
            Backend::Memory(map) => {
                lock(map).remove(user);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
fn lock(
    map: &Mutex<HashMap<String, String>>,
) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
    map.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_round_trip() {
        let secrets = Secrets::in_memory();
        assert_eq!(secrets.get("k").unwrap(), None);
        secrets.set("k", "v").unwrap();
        assert_eq!(secrets.get("k").unwrap().as_deref(), Some("v"));
        secrets.delete("k").unwrap();
        secrets.delete("k").unwrap();
        assert_eq!(secrets.get("k").unwrap(), None);
    }

    #[test]
    fn an_unreadable_entry_is_missing_to_get_and_an_error_to_read() {
        let secrets = Secrets::in_memory();
        secrets.make_unreadable("k");
        assert_eq!(secrets.get("k").unwrap(), None);
        assert!(secrets.read("k").unwrap_err().contains("Could not read"));
    }
}
