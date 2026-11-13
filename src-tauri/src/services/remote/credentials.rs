//! The remote's credential in the OS keychain, as
//! JSON under [`CREDENTIAL_ACCOUNT`]. Long tokens are split across entries
//! on Windows (the Credential Manager caps a secret's size). Never in a
//! config file.
//!
//! Layout: a short credential is one entry, `nandi.remote.credential`. A
//! long one is `nandi.remote.credential.count` (the number of chunks) plus
//! `nandi.remote.credential.0` … `.n-1`, concatenated on read.

use super::{CredentialKind, RemoteCredential};
use crate::services::secrets::Secrets;

/// Keychain account of the remote credential
pub const CREDENTIAL_ACCOUNT: &str = "nandi.remote.credential";

/// Larger JSON is chunked. The Windows Credential Manager caps a blob at
/// 2560 bytes and the keyring crate stores UTF-16 there, so ~1200 chars
/// is the safe size per entry.
const CHUNK_CHARS: usize = 1200;

fn count_account() -> String {
    format!("{CREDENTIAL_ACCOUNT}.count")
}

fn chunk_account(index: usize) -> String {
    format!("{CREDENTIAL_ACCOUNT}.{index}")
}

/// Stores `credential` (replacing any earlier one). A `None` kind clears.
pub fn save(secrets: &Secrets, credential: &RemoteCredential) -> Result<(), String> {
    clear(secrets)?;
    if credential.kind == CredentialKind::None {
        return Ok(());
    }
    let json = serde_json::to_string(credential)
        .map_err(|e| format!("Failed to encode the credential: {e}"))?;
    let chars: Vec<char> = json.chars().collect();
    if chars.len() <= CHUNK_CHARS {
        return secrets.set(CREDENTIAL_ACCOUNT, &json);
    }
    let chunks: Vec<String> = chars
        .chunks(CHUNK_CHARS)
        .map(|chunk| chunk.iter().collect())
        .collect();
    for (index, chunk) in chunks.iter().enumerate() {
        secrets.set(&chunk_account(index), chunk)?;
    }
    // Written last: a half-written credential reads as none
    secrets.set(&count_account(), &chunks.len().to_string())
}

/// The stored credential, if any (an unreadable one is treated as none
/// and logged, never echoed)
pub fn load(secrets: &Secrets) -> Result<Option<RemoteCredential>, String> {
    let json = match secrets.get(&count_account())? {
        Some(count) => {
            let Ok(count) = count.trim().parse::<usize>() else {
                log::warn!("The remote credential's chunk count is unreadable");
                return Ok(None);
            };
            let mut json = String::new();
            for index in 0..count {
                match secrets.get(&chunk_account(index))? {
                    Some(chunk) => json.push_str(&chunk),
                    None => {
                        log::warn!("The remote credential is missing chunk {index}");
                        return Ok(None);
                    }
                }
            }
            json
        }
        None => match secrets.get(CREDENTIAL_ACCOUNT)? {
            Some(json) => json,
            None => return Ok(None),
        },
    };
    match serde_json::from_str::<RemoteCredential>(&json) {
        Ok(credential) if credential.kind != CredentialKind::None => Ok(Some(credential)),
        Ok(_) => Ok(None),
        Err(e) => {
            log::warn!("The stored remote credential is unreadable: {e}");
            Ok(None)
        }
    }
}

/// Removes every entry of the credential (single or chunked)
pub fn clear(secrets: &Secrets) -> Result<(), String> {
    if let Some(count) = secrets
        .get(&count_account())?
        .and_then(|c| c.trim().parse::<usize>().ok())
    {
        for index in 0..count {
            secrets.delete(&chunk_account(index))?;
        }
    }
    secrets.delete(&count_account())?;
    secrets.delete(CREDENTIAL_ACCOUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> RemoteCredential {
        RemoteCredential {
            kind: CredentialKind::Token,
            token: Some(value.to_string()),
            username: None,
            ssh_key_path: None,
        }
    }

    #[test]
    fn round_trips_a_short_credential() {
        let secrets = Secrets::in_memory();
        assert!(load(&secrets).unwrap().is_none());
        let credential = token("ghp_short");
        save(&secrets, &credential).unwrap();
        assert!(secrets.get(CREDENTIAL_ACCOUNT).unwrap().is_some());
        assert!(load(&secrets).unwrap() == Some(credential));
        clear(&secrets).unwrap();
        assert!(load(&secrets).unwrap().is_none());
    }

    #[test]
    fn chunks_a_long_token_and_reassembles_it() {
        let secrets = Secrets::in_memory();
        let long: String = (0..5000u32)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let credential = token(&long);
        save(&secrets, &credential).unwrap();
        assert!(secrets.get(CREDENTIAL_ACCOUNT).unwrap().is_none());
        let count: usize = secrets
            .get(&count_account())
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(count >= 5);
        for index in 0..count {
            let chunk = secrets.get(&chunk_account(index)).unwrap().unwrap();
            assert!(chunk.chars().count() <= CHUNK_CHARS);
        }
        assert!(load(&secrets).unwrap() == Some(credential.clone()));

        // A short one replaces it, leaving no chunks behind
        save(&secrets, &token("short")).unwrap();
        assert!(secrets.get(&count_account()).unwrap().is_none());
        assert!(secrets.get(&chunk_account(0)).unwrap().is_none());
        assert!(load(&secrets).unwrap() == Some(token("short")));

        save(&secrets, &credential).unwrap();
        clear(&secrets).unwrap();
        assert!(secrets.get(&count_account()).unwrap().is_none());
        assert!(secrets.get(&chunk_account(0)).unwrap().is_none());
        assert!(load(&secrets).unwrap().is_none());
    }

    #[test]
    fn a_none_kind_clears() {
        let secrets = Secrets::in_memory();
        save(&secrets, &token("x")).unwrap();
        save(
            &secrets,
            &RemoteCredential {
                kind: CredentialKind::None,
                token: None,
                username: None,
                ssh_key_path: None,
            },
        )
        .unwrap();
        assert!(load(&secrets).unwrap().is_none());
    }
}
