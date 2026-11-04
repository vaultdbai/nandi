//! Paths under `~/.nandi/` and the local config file `~/.nandi/config.json`.
//!
//! `NANDI_HOME` replaces `~/.nandi` (e.g. `NANDI_HOME=~/.nandi-b`), so two
//! instances with separate state can run on one machine.
//!
//! The config holds non-secret settings only; credentials live in the OS
//! keychain.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::env;

const CONFIG_FILE: &str = "config.json";

/// Contents of `~/.nandi/config.json`. Unknown keys are preserved on save.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NandiConfig {
    /// The relay to connect to (`ws://host:8787/relay`), without any
    /// instance token (that stays in `relay_state`). `NANDI_RELAY_URL`
    /// overrides it; see `services::relay`
    #[serde(default, rename = "relay_url", skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    /// The git remote for `~/.nandi/repo`. No secrets: the credential is
    /// in the keychain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<crate::services::remote::RemoteConfig>,
    /// The GitHub sign-in's login, for display (the token is in the
    /// keychain; see `services::auth::github`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_login: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Environment variable replacing `~/.nandi`
pub const HOME_ENV: &str = "NANDI_HOME";

/// The data folder in the user's home
const HOME_DIR: &str = ".nandi";

/// `~/.nandi`, or `NANDI_HOME` when set
pub fn nandi_dir() -> Result<PathBuf, String> {
    resolve_nandi_dir(env::var_os(HOME_ENV), dirs::home_dir())
}

/// Whether `NANDI_HOME` points somewhere other than `~/.nandi`
pub fn has_custom_home() -> bool {
    env::var_os(HOME_ENV).is_some()
}

/// `nandi_home` (with a leading `~` expanded) if set, else the default
/// folder under `home`
fn resolve_nandi_dir(
    nandi_home: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(value) = nandi_home.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if let Ok(rest) = path.strip_prefix("~") {
            let home = home.ok_or("Could not resolve the home directory for NANDI_HOME")?;
            return Ok(home.join(rest));
        }
        return Ok(path);
    }
    home.map(|home| home.join(HOME_DIR))
        .ok_or_else(|| "Could not resolve home directory".to_string())
}

/// Eight hex characters naming this Nandi on the machine, from its folder:
/// two Nandis with separate `NANDI_HOME`s get separate containers. Stable
/// while the folder stays put.
pub fn instance_tag() -> &'static str {
    static TAG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TAG.get_or_init(|| {
        let dir = nandi_dir().unwrap_or_else(|_| PathBuf::from(HOME_DIR));
        tag_for(&dir)
    })
}

fn tag_for(dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// `~/.nandi/local` — machine-local state, never in git: the SQLite
/// projection, logs, agent workdirs, caches
pub fn local_dir() -> Result<PathBuf, String> {
    Ok(nandi_dir()?.join("local"))
}

/// `~/.nandi/local/state.db`
pub fn state_db_path() -> Result<PathBuf, String> {
    Ok(local_dir()?.join("state.db"))
}

/// `~/.nandi/local/logs` — audit logs (llm, egress, triggers)
pub fn logs_dir() -> Result<PathBuf, String> {
    Ok(local_dir()?.join("logs"))
}

/// `~/.nandi/local/work` — agents' ephemeral workdirs
pub fn work_dir() -> Result<PathBuf, String> {
    Ok(local_dir()?.join("work"))
}

/// Loads, applies `change`, and saves the config in `dir`
pub fn update_in(dir: &Path, change: impl FnOnce(&mut NandiConfig)) -> Result<(), String> {
    let mut config = load_from(dir)?;
    change(&mut config);
    save_to(dir, &config)
}

/// A missing config file yields the defaults.
pub fn load_from(dir: &Path) -> Result<NandiConfig, String> {
    let path = dir.join(CONFIG_FILE);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(NandiConfig::default()),
        Err(e) => {
            log::error!("Failed to read {}: {e}", path.display());
            return Err(format!("Failed to read config: {e}"));
        }
    };

    serde_json::from_str(&content).map_err(|e| {
        log::error!("Invalid config at {}: {e}", path.display());
        format!("Invalid config file {}: {e}", path.display())
    })
}

/// Writes via a temp file + rename so a crash never leaves a half-written config.
pub fn save_to(dir: &Path, config: &NandiConfig) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;

    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;

    let path = dir.join(CONFIG_FILE);
    let tmp = dir.join(format!("{CONFIG_FILE}.tmp"));
    fs::write(&tmp, json).map_err(|e| format!("Failed to write config: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to save config: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_tag_differs_per_folder_and_stays_put() {
        let a = tag_for(Path::new("/home/u/.nandi"));
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, tag_for(Path::new("/home/u/.nandi")));
        assert_ne!(a, tag_for(Path::new("/home/u/.nandi-b")));
    }

    #[test]
    fn missing_file_yields_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load_from(tmp.path()).unwrap(), NandiConfig::default());
    }

    #[test]
    fn round_trips_and_preserves_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(CONFIG_FILE),
            r#"{"relay_url":"ws://10.0.0.5:8787/relay","futureSetting":42}"#,
        )
        .unwrap();

        let mut config = load_from(tmp.path()).unwrap();
        assert_eq!(
            config.relay_url.as_deref(),
            Some("ws://10.0.0.5:8787/relay")
        );
        config.relay_url = Some("ws://10.0.0.6:8787/relay".to_string());
        save_to(tmp.path(), &config).unwrap();

        let reloaded = load_from(tmp.path()).unwrap();
        assert_eq!(reloaded, config);
        assert_eq!(reloaded.extra["futureSetting"], 42);
    }

    #[test]
    fn nandi_home_replaces_the_default() {
        let home = Some(PathBuf::from("/home/ada"));
        assert_eq!(
            resolve_nandi_dir(None, home.clone()).unwrap(),
            PathBuf::from("/home/ada/.nandi")
        );
        assert_eq!(
            resolve_nandi_dir(Some("".into()), home.clone()).unwrap(),
            PathBuf::from("/home/ada/.nandi")
        );
        assert_eq!(
            resolve_nandi_dir(Some("/tmp/nandi-b".into()), home.clone()).unwrap(),
            PathBuf::from("/tmp/nandi-b")
        );
        assert_eq!(
            resolve_nandi_dir(Some("~/.nandi-a".into()), home).unwrap(),
            PathBuf::from("/home/ada/.nandi-a")
        );
        assert!(resolve_nandi_dir(None, None).is_err());
    }

    #[test]
    fn relay_url_uses_its_snake_case_key() {
        let tmp = tempfile::tempdir().unwrap();
        update_in(tmp.path(), |config| {
            config.relay_url = Some("ws://10.0.0.5:8787/relay".into());
        })
        .unwrap();
        let raw = fs::read_to_string(tmp.path().join(CONFIG_FILE)).unwrap();
        assert!(
            raw.contains(r#""relay_url": "ws://10.0.0.5:8787/relay""#),
            "{raw}"
        );
        assert_eq!(
            load_from(tmp.path()).unwrap().relay_url.as_deref(),
            Some("ws://10.0.0.5:8787/relay")
        );
    }

    #[test]
    fn rejects_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(CONFIG_FILE), "{not json").unwrap();
        assert!(load_from(tmp.path()).is_err());
    }
}
