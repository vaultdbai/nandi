//! This instance's own keys: an Ed25519 signing key and an X25519 sealing
//! key (`nandi-seal`), one pair per device.
//!
//! They're made the first time this Nandi uses a relay (it connects or
//! enrolls) or the free trial's gateway (`llm_trial`), and kept in the OS keyring under [`DEVICE_KEYS_ENTRY`]. The
//! secret is 86 characters, well under Windows' 2560-byte credential limit,
//! so it's one entry, not chunked. The public half is sent at enrollment and
//! registration, and the relay pins it for this instance ID (the trial
//! gateway pins it too, on first use).
//!
//! A keyring that can't be read is an error, never a reason to make new
//! keys: new keys for the same ID would be refused by the relay
//! (`key_mismatch`) until its operator rekeys it.

use std::sync::Mutex;

use nandi_protocol::PublicKeys;
use nandi_seal::DeviceKeys;

use crate::services::secrets::Secrets;

/// The keyring entry holding this instance's secret keys
pub const DEVICE_KEYS_ENTRY: &str = "nandi.instance.keys";

/// Serializes "read, else make and store" across every caller, so two
/// first uses at once don't make two pairs
static CREATING: Mutex<()> = Mutex::new(());

/// This instance's public keys, from the keyring (made and stored on first
/// use). Cheap after the first call.
pub struct DeviceKeyring {
    secrets: Secrets,
    public: Mutex<Option<PublicKeys>>,
}

impl DeviceKeyring {
    pub fn new(secrets: Secrets) -> Self {
        Self {
            secrets,
            public: Mutex::new(None),
        }
    }

    /// The public keys, making the pair if there is none yet
    pub fn public_keys(&self) -> Result<PublicKeys, String> {
        let mut cached = self.public.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(keys) = cached.as_ref() {
            return Ok(keys.clone());
        }
        let keys = load_or_create(&self.secrets)?.public_keys().clone();
        *cached = Some(keys.clone());
        Ok(keys)
    }

    /// The secret keys, to seal and open with (made if there are none)
    pub fn device_keys(&self) -> Result<DeviceKeys, String> {
        let keys = load_or_create(&self.secrets)?;
        *self.public.lock().unwrap_or_else(|e| e.into_inner()) = Some(keys.public_keys().clone());
        Ok(keys)
    }
}

/// This instance's keys: the stored pair, or a new one stored now
pub fn load_or_create(secrets: &Secrets) -> Result<DeviceKeys, String> {
    let _creating = CREATING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(secret) = secrets.read(DEVICE_KEYS_ENTRY)? {
        return DeviceKeys::import_secret(&secret).map_err(|e| {
            format!("This device's keys in the keyring ({DEVICE_KEYS_ENTRY}) can't be read: {e}")
        });
    }
    let keys =
        DeviceKeys::generate().map_err(|e| format!("Couldn't make this device's keys: {e}"))?;
    secrets.set(DEVICE_KEYS_ENTRY, &keys.export_secret())?;
    log::info!("Made this device's keys ({})", keys.key_id());
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_made_once_and_kept_in_the_keyring() {
        let secrets = Secrets::in_memory();
        assert_eq!(secrets.get(DEVICE_KEYS_ENTRY).unwrap(), None);
        let first = DeviceKeyring::new(secrets.clone()).public_keys().unwrap();
        assert!(first.is_well_formed());
        assert!(secrets.get(DEVICE_KEYS_ENTRY).unwrap().is_some());
        // Another reader of the same keyring gets the same pair
        let again = DeviceKeyring::new(secrets.clone()).public_keys().unwrap();
        assert_eq!(again, first);
        assert_eq!(load_or_create(&secrets).unwrap().public_keys(), &first);
    }

    #[test]
    fn an_unreadable_keyring_never_makes_new_keys() {
        let secrets = Secrets::in_memory();
        secrets.make_unreadable(DEVICE_KEYS_ENTRY);
        assert!(DeviceKeyring::new(secrets.clone()).public_keys().is_err());
        secrets.set(DEVICE_KEYS_ENTRY, "not a key").unwrap();
        let error = load_or_create(&secrets).unwrap_err();
        assert!(error.contains("can't be read"), "{error}");
        assert_eq!(
            secrets.get(DEVICE_KEYS_ENTRY).unwrap().as_deref(),
            Some("not a key")
        );
    }
}
