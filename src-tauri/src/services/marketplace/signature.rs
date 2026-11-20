//! Publisher signatures (Ed25519).
//!
//! A publisher has a key pair. The public key is in the registry's
//! `publishers/<handle>.json` (`"public_key": "ed25519:<base64>"`); which
//! key an entry is actually checked against is `trust`'s rule. For each
//! release they sign a short statement naming exactly what they vouch for,
//! and put it in the entry with the commit it names:
//!
//! ```text
//! nandi-signature-v1
//! agent                         # or pack
//! arxiv-researcher
//! 1.2.0
//! e5d4c3b2…                     # the full commit SHA of the release tag
//! ```
//!
//! ```json
//! { "id": "arxiv-researcher", "latest_version": "1.2.0",
//!   "commit": "e5d4c3b2…", "signature": "ed25519:<base64>" }
//! ```
//!
//! The chain is: publisher key → signature → commit → files. A listing can
//! check it without downloading anything but the publisher file. An install
//! also checks that the tag still resolves to that commit, and refuses if
//! it doesn't (a moved tag). The default registry refuses unsigned entries;
//! other registries' still install, with a "Publisher not verified" warning
//! (`trust::admit`).
//!
//! `nandi keygen` / `nandi sign` (see `cli.rs`) make keys and signatures.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::services::secrets::Secrets;

const PREFIX: &str = "ed25519:";
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// What a publisher signs for one release.
pub fn message(kind: &str, id: &str, version: &str, commit: &str) -> String {
    format!(
        "nandi-signature-v1\n{kind}\n{id}\n{}\n{}\n",
        version.trim_start_matches('v'),
        commit.to_ascii_lowercase()
    )
}

fn decode(value: &str, what: &str) -> Result<Vec<u8>, String> {
    let encoded = value
        .trim()
        .strip_prefix(PREFIX)
        .ok_or_else(|| format!("The {what} must start with '{PREFIX}'"))?;
    B64.decode(encoded)
        .map_err(|_| format!("The {what} isn't valid base64"))
}

pub fn parse_public_key(value: &str) -> Result<VerifyingKey, String> {
    let bytes: [u8; 32] = decode(value, "public key")?
        .try_into()
        .map_err(|_| "An Ed25519 public key is 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "Not a valid Ed25519 public key".to_string())
}

fn parse_signature(value: &str) -> Result<Signature, String> {
    let bytes: [u8; 64] = decode(value, "signature")?
        .try_into()
        .map_err(|_| "An Ed25519 signature is 64 bytes".to_string())?;
    Ok(Signature::from_bytes(&bytes))
}

/// Whether `signature` is `public_key`'s signature of this release.
/// Anything malformed is simply not verified.
pub fn verify(
    public_key: &str,
    signature: &str,
    kind: &str,
    id: &str,
    version: &str,
    commit: &str,
) -> bool {
    let (Ok(key), Ok(signature)) = (parse_public_key(public_key), parse_signature(signature))
    else {
        return false;
    };
    key.verify_strict(message(kind, id, version, commit).as_bytes(), &signature)
        .is_ok()
}

/// A new key pair: (secret key file contents, public key for
/// `publishers/<handle>.json`).
pub fn generate() -> (String, String) {
    let key = SigningKey::generate(&mut rand_core::OsRng);
    (
        format!("{PREFIX}{}\n", B64.encode(key.to_bytes())),
        format!("{PREFIX}{}", B64.encode(key.verifying_key().to_bytes())),
    )
}

/// Signs a release with a secret key (as written by `generate`).
pub fn sign(
    secret_key: &str,
    kind: &str,
    id: &str,
    version: &str,
    commit: &str,
) -> Result<String, String> {
    let bytes: [u8; 32] = decode(secret_key, "secret key")?
        .try_into()
        .map_err(|_| "An Ed25519 secret key is 32 bytes".to_string())?;
    let key = SigningKey::from_bytes(&bytes);
    let signature = key.sign(message(kind, id, version, commit).as_bytes());
    Ok(format!("{PREFIX}{}", B64.encode(signature.to_bytes())))
}

/// The public key belonging to a secret key.
pub fn public_key_of(secret_key: &str) -> Result<String, String> {
    let bytes: [u8; 32] = decode(secret_key, "secret key")?
        .try_into()
        .map_err(|_| "An Ed25519 secret key is 32 bytes".to_string())?;
    Ok(format!(
        "{PREFIX}{}",
        B64.encode(SigningKey::from_bytes(&bytes).verifying_key().to_bytes())
    ))
}

/// `~/.nandi/keys/publisher.key`
pub(crate) fn default_key_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::utils::nandi_config::nandi_dir()?
        .join("keys")
        .join("publisher.key"))
}

/// Where a publisher's secret key lives: a file, or an entry in the OS
/// keychain (`--key keyring:<name>`), so it never has to sit on disk
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyLocation {
    File(std::path::PathBuf),
    Keychain(String),
}

/// The keychain entry of signing key `name`
fn keychain_entry(name: &str) -> String {
    format!("nandi.signing.{name}")
}

impl KeyLocation {
    /// `--key` as given: `keyring:<name>` or a path; none: the default file
    pub fn parse(key: Option<&str>) -> Result<Self, String> {
        let Some(key) = key else {
            return Ok(Self::File(default_key_path()?));
        };
        match key.strip_prefix("keyring:") {
            Some(name)
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
            {
                Ok(Self::Keychain(name.to_string()))
            }
            Some(_) => {
                Err("keyring:<NAME> needs a name of letters, digits, '-' or '_'".to_string())
            }
            None => Ok(Self::File(std::path::PathBuf::from(key))),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Keychain(name) => format!(
                "{} (entry {})",
                crate::services::secrets::keychain_name(),
                keychain_entry(name)
            ),
        }
    }

    fn read(&self, secrets: &Secrets) -> Result<String, String> {
        match self {
            Self::File(path) => std::fs::read_to_string(path)
                .map_err(|e| format!("Can't read the key {}: {e}", path.display())),
            Self::Keychain(name) => secrets
                .read(&keychain_entry(name))?
                .ok_or_else(|| format!("No signing key '{name}' in {}", self.describe())),
        }
    }

    /// Stores a new key; never overwrites one
    fn write_new(&self, secret: &str, secrets: &Secrets) -> Result<(), String> {
        let taken = format!(
            "{} already holds a key; refusing to overwrite it. Use --key with a new name or path.",
            self.describe()
        );
        match self {
            Self::File(path) => {
                if path.exists() {
                    return Err(taken);
                }
                crate::services::repos::fs_atomic::write_atomic(path, secret.as_bytes())
                    .map_err(|e| format!("{}: {e}", path.display()))
            }
            Self::Keychain(name) => {
                if secrets.read(&keychain_entry(name))?.is_some() {
                    return Err(taken);
                }
                secrets.set(&keychain_entry(name), secret)
            }
        }
    }
}

/// Runs `nandi keygen` / `nandi sign`. Returns the exit code.
pub fn run_cli(command: &crate::cli::PublisherCommand) -> i32 {
    let secrets = Secrets::keychain(crate::services::APP_IDENTIFIER);
    match run(command, &secrets) {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn run(command: &crate::cli::PublisherCommand, secrets: &Secrets) -> Result<String, String> {
    use crate::cli::PublisherCommand;
    match command {
        PublisherCommand::Keygen { key, import } => {
            let location = KeyLocation::parse(key.as_deref())?;
            let (secret, public) = match import {
                None => generate(),
                Some(from) => {
                    if matches!(location, KeyLocation::File(_)) {
                        return Err(
                            "--import moves a key into the keychain: give --key keyring:<NAME>"
                                .to_string(),
                        );
                    }
                    let secret = std::fs::read_to_string(from)
                        .map_err(|e| format!("Can't read the key {from}: {e}"))?
                        .trim()
                        .to_string();
                    let public = public_key_of(&secret)?;
                    (secret, public)
                }
            };
            location.write_new(&secret, secrets)?;
            let next = match import {
                Some(from) => format!(
                    "
Back up {from} offline (an encrypted drive), then delete it from this computer.
Sign with --key {}",
                    key.as_deref().unwrap_or_default()
                ),
                None => String::new(),
            };
            Ok(format!(
                "Secret key stored in {} (keep it private and backed up).
Public key for publishers/<handle>.json:
  \"public_key\": \"{public}\"{next}",
                location.describe()
            ))
        }
        PublisherCommand::Sign {
            kind,
            id,
            release,
            commit,
            repo,
            key,
        } => {
            let secret = KeyLocation::parse(key.as_deref())?.read(secrets)?;
            let secret = secret.trim();
            let version = release.trim_start_matches('v');
            let commit = match (commit, repo) {
                (Some(commit), _) => commit.to_ascii_lowercase(),
                (None, Some(repo)) => {
                    super::source::GitSource::new(repo, None)
                        .resolve(&format!("v{version}"))?
                        .commit
                }
                (None, None) => return Err("Give --commit or --repo".to_string()),
            };
            if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("'{commit}' is not a full 40-character commit SHA"));
            }
            let signature = sign(secret, kind, id, version, &commit)?;
            Ok(format!(
                "\"commit\": \"{commit}\",\n\"signature\": \"{signature}\"\n(public key {})",
                public_key_of(secret)?
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "e5d4c3b2a1f0e5d4c3b2a1f0e5d4c3b2a1f0e5d4";

    fn keygen(key: &str, import: Option<&str>) -> crate::cli::PublisherCommand {
        crate::cli::PublisherCommand::Keygen {
            key: Some(key.to_string()),
            import: import.map(str::to_string),
        }
    }

    fn sign_with(key: &str) -> crate::cli::PublisherCommand {
        crate::cli::PublisherCommand::Sign {
            kind: "agent".into(),
            id: "arxiv".into(),
            release: "1.2.0".into(),
            commit: Some(COMMIT.into()),
            repo: None,
            key: Some(key.to_string()),
        }
    }

    #[test]
    fn a_key_in_the_keychain_signs_without_a_file() {
        let secrets = Secrets::in_memory();
        let made = run(&keygen("keyring:publisher", None), &secrets).unwrap();
        assert!(made.contains("nandi.signing.publisher"), "{made}");
        let signed = run(&sign_with("keyring:publisher"), &secrets).unwrap();
        assert!(signed.contains("\"signature\": \"ed25519:"), "{signed}");
        let other = run(&sign_with("keyring:other"), &secrets).unwrap_err();
        assert!(other.contains("No signing key 'other'"), "{other}");
    }

    #[test]
    fn import_moves_a_key_file_into_the_keychain_and_never_overwrites() {
        let secrets = Secrets::in_memory();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("publisher.key");
        let (secret, public) = generate();
        std::fs::write(
            &file,
            format!(
                "{secret}
"
            ),
        )
        .unwrap();
        let from = file.to_str().unwrap();

        let out = run(&keygen("keyring:publisher", Some(from)), &secrets).unwrap();
        assert!(out.contains(&public) && out.contains("delete it"), "{out}");
        // The same public key, from the keychain
        assert!(run(&sign_with("keyring:publisher"), &secrets)
            .unwrap()
            .contains(&public));
        // A second import or keygen into the same entry is refused
        let again = run(&keygen("keyring:publisher", Some(from)), &secrets).unwrap_err();
        assert!(again.contains("refusing to overwrite"), "{again}");
        // Importing into a file isn't a move
        let to_file = dir.path().join("copy.key");
        let err = run(&keygen(to_file.to_str().unwrap(), Some(from)), &secrets).unwrap_err();
        assert!(err.contains("keyring:<NAME>"), "{err}");
        // Not a key: refused before anything is stored
        let junk = dir.path().join("junk.key");
        std::fs::write(&junk, "not a key").unwrap();
        assert!(run(
            &keygen("keyring:junk", Some(junk.to_str().unwrap())),
            &secrets
        )
        .is_err());
        assert!(run(&sign_with("keyring:junk"), &secrets).is_err());
    }

    #[test]
    fn key_locations_parse() {
        assert_eq!(
            KeyLocation::parse(Some("keyring:nest-publisher")).unwrap(),
            KeyLocation::Keychain("nest-publisher".into())
        );
        assert!(KeyLocation::parse(Some("keyring:")).is_err());
        assert!(KeyLocation::parse(Some("keyring:a b")).is_err());
        assert_eq!(
            KeyLocation::parse(Some("C:/keys/nest publisher.key")).unwrap(),
            KeyLocation::File("C:/keys/nest publisher.key".into())
        );
    }

    #[test]
    fn verifies_a_valid_signature() {
        let (secret, public) = generate();
        let signature = sign(&secret, "agent", "arxiv", "1.2.0", COMMIT).unwrap();
        assert!(verify(
            &public, &signature, "agent", "arxiv", "1.2.0", COMMIT
        ));
        // `v1.2.0` and upper-case SHAs name the same release
        assert!(verify(
            &public,
            &signature,
            "agent",
            "arxiv",
            "v1.2.0",
            &COMMIT.to_uppercase()
        ));
        assert_eq!(public_key_of(&secret).unwrap(), public);
    }

    #[test]
    fn rejects_tampered_releases_and_other_keys() {
        let (secret, public) = generate();
        let signature = sign(&secret, "agent", "arxiv", "1.2.0", COMMIT).unwrap();
        let other_commit = COMMIT.replace('e', "f");
        for (kind, id, version, commit) in [
            ("agent", "arxiv", "1.2.0", other_commit.as_str()),
            ("agent", "arxiv", "1.3.0", COMMIT),
            ("agent", "evil", "1.2.0", COMMIT),
            ("pack", "arxiv", "1.2.0", COMMIT),
        ] {
            assert!(
                !verify(&public, &signature, kind, id, version, commit),
                "{kind} {id} {version}"
            );
        }
        let (_, stranger) = generate();
        assert!(!verify(
            &stranger, &signature, "agent", "arxiv", "1.2.0", COMMIT
        ));

        // Flip one bit of the signature
        let mut bytes = B64.decode(signature.strip_prefix(PREFIX).unwrap()).unwrap();
        bytes[0] ^= 1;
        let tampered = format!("{PREFIX}{}", B64.encode(bytes));
        assert!(!verify(
            &public, &tampered, "agent", "arxiv", "1.2.0", COMMIT
        ));
        // Malformed input is "not verified", never a panic
        assert!(!verify(
            "ed25519:short",
            &signature,
            "agent",
            "arxiv",
            "1.2.0",
            COMMIT
        ));
        assert!(!verify(
            &public, "rsa:abc", "agent", "arxiv", "1.2.0", COMMIT
        ));
    }
}
