//! Secrets in the OS keychain (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux), via the `keyring` crate directly so services
//! don't need a Tauri `AppHandle`.
//!
//! Entries live under the app identifier as the keychain *service*.
//!
//! **Large values are split.** Windows Credential Manager holds at most
//! 2,560 bytes per credential, which `keyring` fills with UTF-16 (1,280
//! code units). A Microsoft sign-in (access + refresh token) is larger. A
//! value over [`MAX_UNITS`] is stored as chunks at `<entry>#1`,
//! `<entry>#2`, …, and `<entry>` holds only a header with the count
//! (`nandi:chunks:<n>`). New chunks are written before the header, and
//! chunks a shorter value no longer needs are removed after it, so a
//! reader sees the old value or the new one. Every platform does the same,
//! so a secret reads back the same everywhere.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Arc, Mutex};

/// UTF-16 code units one credential holds (Windows: 2,560 bytes)
pub const MAX_UNITS: usize = 1280;
/// UTF-16 code units per chunk, with room to spare
const CHUNK_UNITS: usize = 1200;
/// The first entry of a split value
const HEADER: &str = "nandi:chunks:";

#[derive(Clone)]
enum Backend {
    Keychain {
        service: String,
    },
    /// For tests: never touches the real keychain, and refuses values over
    /// Windows' limit like Credential Manager does
    #[cfg(test)]
    Memory(Arc<Mutex<HashMap<String, String>>>),
}

#[derive(Clone)]
pub struct Secrets(Backend);

const OPEN_FAILED: &str = "Failed to open the system keychain";
/// For tests: a stored value that reads as an error
#[cfg(test)]
const UNREADABLE: &str = "[unreadable in tests]";

/// The keychain's name, for messages
pub fn keychain_name() -> &'static str {
    if cfg!(windows) {
        "Windows Credential Manager"
    } else if cfg!(target_os = "macos") {
        "the macOS Keychain"
    } else {
        "the system keyring"
    }
}

fn chunk_entry(user: &str, index: usize) -> String {
    format!("{user}#{index}")
}

fn chunk_count(header: &str) -> Option<usize> {
    header.strip_prefix(HEADER)?.parse().ok().filter(|n| *n > 0)
}

/// `secret` cut into pieces of at most [`CHUNK_UNITS`] UTF-16 code units,
/// never inside a character
fn split(secret: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut units = 0;
    for c in secret.chars() {
        let width = c.len_utf16();
        if units + width > CHUNK_UNITS {
            chunks.push(std::mem::take(&mut current));
            units = 0;
        }
        current.push(c);
        units += width;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn fits(secret: &str) -> bool {
    secret.encode_utf16().count() <= MAX_UNITS && !secret.starts_with(HEADER)
}

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

    // --- One credential each -------------------------------------------

    fn raw_read(&self, user: &str) -> Result<Option<String>, String> {
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

    fn raw_set(&self, user: &str, secret: &str) -> Result<(), String> {
        let failed =
            |e: &dyn std::fmt::Display| format!("Couldn't save to {}: {e}", keychain_name());
        match &self.0 {
            Backend::Keychain { service } => keyring::Entry::new(service, user)
                .and_then(|entry| entry.set_password(secret))
                .map_err(|e| failed(&e)),
            #[cfg(test)]
            Backend::Memory(map) => {
                if secret.encode_utf16().count() > MAX_UNITS {
                    return Err(failed(
                        &"Attribute 'password' is longer than platform limit of 2560 chars",
                    ));
                }
                lock(map).insert(user.to_string(), secret.to_string());
                Ok(())
            }
        }
    }

    fn raw_delete(&self, user: &str) -> Result<(), String> {
        match &self.0 {
            Backend::Keychain { service } => {
                match keyring::Entry::new(service, user).and_then(|e| e.delete_credential()) {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(format!("Failed to remove from {}: {e}", keychain_name())),
                }
            }
            #[cfg(test)]
            Backend::Memory(map) => {
                lock(map).remove(user);
                Ok(())
            }
        }
    }

    /// How many chunks `user` is split into now (0: not split)
    fn stored_chunks(&self, user: &str) -> usize {
        self.raw_read(user)
            .ok()
            .flatten()
            .and_then(|value| chunk_count(&value))
            .unwrap_or(0)
    }

    // --- Values ------------------------------------------------------------

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
        let Some(value) = self.raw_read(user)? else {
            return Ok(None);
        };
        let Some(count) = chunk_count(&value) else {
            return Ok(Some(value));
        };
        let mut joined = String::new();
        for index in 1..=count {
            match self.raw_read(&chunk_entry(user, index))? {
                Some(chunk) => joined.push_str(&chunk),
                None => {
                    return Err(format!(
                        "Could not read keychain entry '{user}': part {index} of {count} is missing"
                    ))
                }
            }
        }
        Ok(Some(joined))
    }

    /// For tests: the entry exists but can't be read, like a locked keychain
    #[cfg(test)]
    pub fn make_unreadable(&self, user: &str) {
        if let Backend::Memory(map) = &self.0 {
            lock(map).insert(user.to_string(), UNREADABLE.to_string());
        }
    }

    pub fn set(&self, user: &str, secret: &str) -> Result<(), String> {
        let before = self.stored_chunks(user);
        let after = if fits(secret) {
            self.raw_set(user, secret)?;
            0
        } else {
            let chunks = split(secret);
            for (index, chunk) in chunks.iter().enumerate() {
                self.raw_set(&chunk_entry(user, index + 1), chunk)?;
            }
            self.raw_set(user, &format!("{HEADER}{}", chunks.len()))?;
            chunks.len()
        };
        // Pieces the new value doesn't use
        for index in after + 1..=before {
            if let Err(e) = self.raw_delete(&chunk_entry(user, index)) {
                log::warn!("{e}");
            }
        }
        Ok(())
    }

    /// Removes the secret and all its pieces; a missing entry is not an
    /// error.
    pub fn delete(&self, user: &str) -> Result<(), String> {
        let chunks = self.stored_chunks(user);
        self.raw_delete(user)?;
        for index in 1..=chunks {
            self.raw_delete(&chunk_entry(user, index))?;
        }
        Ok(())
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

    fn stored(secrets: &Secrets) -> Vec<String> {
        let Backend::Memory(map) = &secrets.0 else {
            unreachable!()
        };
        let mut keys: Vec<String> = lock(map).keys().cloned().collect();
        keys.sort();
        keys
    }

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

    #[test]
    fn the_test_store_has_windows_limit() {
        let secrets = Secrets::in_memory();
        let error = secrets
            .raw_set("k", &"x".repeat(MAX_UNITS + 1))
            .unwrap_err();
        assert!(error.contains("platform limit"), "{error}");
        secrets.raw_set("k", &"x".repeat(MAX_UNITS)).unwrap();
    }

    #[test]
    fn a_6_kb_token_round_trips_in_pieces() {
        let secrets = Secrets::in_memory();
        let token = format!(
            "{{\"access_token\":\"{}\",\"refresh_token\":\"{}\"}}",
            "EwB4A8l6BAAU".repeat(450),
            "M.C556_BAY.".repeat(100)
        );
        assert!(token.len() > 6_000);
        secrets.set("nandi.email.outlook.oauth", &token).unwrap();
        assert_eq!(
            secrets.get("nandi.email.outlook.oauth").unwrap().as_deref(),
            Some(token.as_str())
        );
        let keys = stored(&secrets);
        assert!(keys.len() > 2, "{keys:?}");
        assert!(keys.contains(&"nandi.email.outlook.oauth#1".to_string()));
    }

    #[test]
    fn delete_removes_every_piece() {
        let secrets = Secrets::in_memory();
        secrets.set("k", &"a".repeat(5_000)).unwrap();
        secrets.delete("k").unwrap();
        assert!(stored(&secrets).is_empty());
        assert_eq!(secrets.get("k").unwrap(), None);
    }

    #[test]
    fn a_smaller_value_removes_the_pieces_it_no_longer_needs() {
        let secrets = Secrets::in_memory();
        secrets.set("k", &"a".repeat(5_000)).unwrap();
        secrets.set("k", &"b".repeat(1_500)).unwrap();
        assert_eq!(stored(&secrets), ["k", "k#1", "k#2"]);
        assert_eq!(secrets.get("k").unwrap().unwrap(), "b".repeat(1_500));
        secrets.set("k", "short").unwrap();
        assert_eq!(stored(&secrets), ["k"]);
        assert_eq!(secrets.get("k").unwrap().as_deref(), Some("short"));
    }

    #[test]
    fn pieces_never_split_a_character() {
        let secrets = Secrets::in_memory();
        // Two UTF-16 units each
        let wide = "😀".repeat(1_000);
        secrets.set("k", &wide).unwrap();
        assert_eq!(secrets.get("k").unwrap().unwrap(), wide);
    }

    #[test]
    fn a_missing_piece_is_an_error_not_a_wrong_value() {
        let secrets = Secrets::in_memory();
        secrets.set("k", &"a".repeat(3_000)).unwrap();
        secrets.raw_delete("k#2").unwrap();
        assert!(secrets.read("k").unwrap_err().contains("part 2 of 3"));
        assert_eq!(secrets.get("k").unwrap(), None);
    }
}
