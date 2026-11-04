//! Publisher signatures (Ed25519).
//!
//! A publisher has a key pair. The public key is in the registry's
//! `publishers/<handle>.json` (`"public_key": "ed25519:<base64>"`). For each
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
//! it doesn't (a moved tag). Unsigned entries can still be installed, with
//! a "Publisher not verified" warning.
//!
//! `nandi keygen` / `nandi sign` (see `cli.rs`) make keys and signatures.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

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

/// Runs `nandi keygen` / `nandi sign`. Returns the exit code.
pub fn run_cli(command: &crate::cli::PublisherCommand) -> i32 {
    match run(command) {
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

fn run(command: &crate::cli::PublisherCommand) -> Result<String, String> {
    use crate::cli::PublisherCommand;
    match command {
        PublisherCommand::Keygen { key } => {
            let path = key
                .as_ref()
                .map_or_else(default_key_path, |k| Ok(std::path::PathBuf::from(k)))?;
            if path.exists() {
                return Err(format!(
                    "{} already exists; refusing to overwrite a key. Use --key <PATH> for a new one.",
                    path.display()
                ));
            }
            let (secret, public) = generate();
            crate::services::repos::fs_atomic::write_atomic(&path, secret.as_bytes())
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(format!(
                "Secret key written to {} (keep it private and backed up).\nPublic key for publishers/<handle>.json:\n  \"public_key\": \"{public}\"",
                path.display()
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
            let path = key
                .as_ref()
                .map_or_else(default_key_path, |k| Ok(std::path::PathBuf::from(k)))?;
            let secret = std::fs::read_to_string(&path)
                .map_err(|e| format!("Can't read the key {}: {e}", path.display()))?;
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
            let signature = sign(&secret, kind, id, version, &commit)?;
            Ok(format!(
                "\"commit\": \"{commit}\",\n\"signature\": \"{signature}\"\n(public key {})",
                public_key_of(&secret)?
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "e5d4c3b2a1f0e5d4c3b2a1f0e5d4c3b2a1f0e5d4";

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
