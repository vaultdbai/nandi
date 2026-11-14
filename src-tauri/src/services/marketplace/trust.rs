//! Which key a publisher's signatures are checked against.
//!
//! - The first-party publisher's key ships in the app
//!   ([`FIRST_PARTY_PUBLIC_KEY`]). Its releases are checked against that
//!   constant only, never against a `publishers/` file fetched from a
//!   registry, so a registry compromise can't swap the key and re-sign.
//! - Any other publisher's key comes from `publishers/<handle>.json` in the
//!   registry that lists the entry, and is pinned the first time one of
//!   their releases installs (trust on first use, kept in settings). A
//!   different key later refuses verification and names both fingerprints.
//!   Nothing re-pins on its own: removing the registry forgets its pins.
//!
//! Whether an install or update may go ahead ([`admit`], [`vouch`]):
//!
//! | Found              | Default registry   | Other registries      |
//! | ------------------ | ------------------ | --------------------- |
//! | Verified           | installs, verified | installs, verified    |
//! | Unsigned, bad      | refused            | installs, unverified  |
//! | Key changed        | refused            | refused               |
//! | Couldn't check     | refused: try again | refused: try again    |
//! | A branch, not tag  | refused            | allowed               |
//!
//! "Couldn't check" is never recorded as unverified: a signed release
//! would stay "unverified" for good.
//!
//! Everything here is pure; `mod.rs` fetches the files and saves the pins.
//! Rotating the shipped key: `docs/developer/marketplace.md`.

use sha2::{Digest, Sha256};

use super::manifest::split_tag;
use super::signature;
use crate::services::domain::settings::{normalize_registry_url, DEFAULT_REGISTRY};

/// The first-party publisher: their releases are checked against the
/// shipped key.
pub const FIRST_PARTY_PUBLISHER: &str = "vaultdbai";

/// The first-party publisher's Ed25519 public key, as published in
/// `publishers/vaultdbai.json` of the default registry.
pub const FIRST_PARTY_PUBLIC_KEY: &str = "ed25519:yKoKXx6WoM+5igtYB0bCkTTwihr6P+kiz6D40VgDaTk=";

/// The key a publisher's releases are checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherKey {
    /// The first-party key shipped in the app
    Shipped,
    /// The registry names the key pinned on an earlier install
    Pinned(String),
    /// The registry names a key seen for the first time: it's pinned when
    /// one of their releases installs
    FirstUse(String),
    /// The registry names another key than the one pinned
    Changed { pinned: String, offered: String },
    /// The publisher's file names no key: they don't sign
    NoKey,
    /// The publisher's file couldn't be fetched (why): try again later
    Unreachable(String),
}

impl PublisherKey {
    /// The shipped key, when `publisher` is the first party. Nothing needs
    /// fetching then.
    pub fn shipped(publisher: &str) -> Option<Self> {
        publisher
            .eq_ignore_ascii_case(FIRST_PARTY_PUBLISHER)
            .then_some(Self::Shipped)
    }

    /// The key from a registry's `publishers/<handle>.json` (`fetched`: the
    /// key it names, `None` when it names none, or why it couldn't be
    /// fetched), held against the key pinned for that publisher.
    pub fn from_registry(pinned: Option<&str>, fetched: Result<Option<String>, String>) -> Self {
        match (pinned, fetched) {
            (_, Err(why)) => Self::Unreachable(why),
            (_, Ok(None)) => Self::NoKey,
            (None, Ok(Some(offered))) => Self::FirstUse(offered),
            (Some(pinned), Ok(Some(offered))) if pinned == offered => Self::Pinned(offered),
            (Some(pinned), Ok(Some(offered))) => Self::Changed {
                pinned: pinned.to_string(),
                offered,
            },
        }
    }
}

/// One release, as a registry entry names it.
#[derive(Debug, Clone, Copy)]
pub struct Release<'a> {
    /// `agent`, `pack` or `connector`
    pub kind: &'a str,
    pub id: &'a str,
    pub version: &'a str,
    pub commit: Option<&'a str>,
    pub signature: Option<&'a str>,
}

/// What checking a release's signature found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureCheck {
    /// Signed by the trusted key. `pin`: the key to pin when it installs
    /// (seen for the first time)
    Verified { pin: Option<String> },
    /// The entry carries no signature (or no commit), or its publisher
    /// doesn't sign
    Unsigned,
    /// The signature doesn't match the trusted key
    BadSignature,
    /// The registry offers another key than the one pinned
    KeyChanged { pinned: String, offered: String },
    /// The publisher's key couldn't be fetched (why)
    CouldNotCheck(String),
}

impl SignatureCheck {
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }
}

/// Checks `release`'s signature against `key`.
pub fn check(key: &PublisherKey, release: &Release<'_>) -> SignatureCheck {
    let (Some(signature), Some(commit)) = (release.signature, release.commit) else {
        return SignatureCheck::Unsigned;
    };
    let (trusted, pin) = match key {
        PublisherKey::Shipped => (FIRST_PARTY_PUBLIC_KEY, None),
        PublisherKey::Pinned(key) => (key.as_str(), None),
        PublisherKey::FirstUse(key) => (key.as_str(), Some(key.clone())),
        PublisherKey::Changed { pinned, offered } => {
            return SignatureCheck::KeyChanged {
                pinned: pinned.clone(),
                offered: offered.clone(),
            }
        }
        PublisherKey::NoKey => return SignatureCheck::Unsigned,
        PublisherKey::Unreachable(why) => return SignatureCheck::CouldNotCheck(why.clone()),
    };
    if signature::verify(
        trusted,
        signature,
        release.kind,
        release.id,
        release.version,
        commit,
    ) {
        SignatureCheck::Verified { pin }
    } else {
        SignatureCheck::BadSignature
    }
}

/// Where an install or update comes from. The default registry's entries
/// must be signed release tags; other registries' may be unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    DefaultRegistry,
    OtherRegistry,
}

impl Origin {
    pub fn of(registry_url: &str) -> Self {
        if normalize_registry_url(registry_url) == DEFAULT_REGISTRY {
            Self::DefaultRegistry
        } else {
            Self::OtherRegistry
        }
    }
}

/// Whether `name`'s release at `git_ref` may install, given what checking
/// its signature found. `Ok(verified)`, or why it's refused.
pub fn admit(
    origin: Origin,
    name: &str,
    git_ref: &str,
    check: &SignatureCheck,
) -> Result<bool, String> {
    match check {
        SignatureCheck::CouldNotCheck(why) => {
            return Err(format!(
                "Couldn't check {name}'s signature right now ({why}). Nothing was installed. Try again later."
            ))
        }
        SignatureCheck::KeyChanged { pinned, offered } => {
            return Err(key_changed_warning(name, pinned, offered))
        }
        _ => {}
    }
    if origin == Origin::OtherRegistry {
        return Ok(check.is_verified());
    }
    if split_tag(git_ref).is_none() {
        return Err(format!(
            "Refusing to install {name}: the default registry must name a release tag, but its entry names '{git_ref}'."
        ));
    }
    match check {
        SignatureCheck::Verified { .. } => Ok(true),
        SignatureCheck::Unsigned => Err(format!(
            "Refusing to install {name}: its entry in the default registry isn't signed, and everything there must be."
        )),
        _ => Err(format!(
            "Refusing to install {name}: its signature doesn't match its publisher's key. The registry entry may have been tampered with."
        )),
    }
}

/// A registry's current entry for an installed item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedRelease {
    pub version: String,
    pub git_ref: String,
    pub commit: Option<String>,
}

/// What the registry vouches for about the release an update applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VouchedRelease {
    /// The commit the release must resolve to (`None`: nothing pins it)
    pub commit: Option<String>,
    /// Its signature verified: recorded as `signature_verified`
    pub verified: bool,
}

/// The registry entry an update of `name` to `new_ref` must match:
/// `Some(entry)` when the registry lists that release. From the default
/// registry an update must be exactly the release it lists; from another
/// registry a release it doesn't list is `None`, and installs unverified.
pub fn update_target<'a>(
    origin: Origin,
    name: &str,
    new_ref: &str,
    listed: Option<&'a ListedRelease>,
) -> Result<Option<&'a ListedRelease>, String> {
    match (origin, listed) {
        (_, Some(entry)) if entry.git_ref == new_ref => Ok(Some(entry)),
        (Origin::DefaultRegistry, Some(entry)) => Err(format!(
            "Refusing to update {name} to {new_ref}: the default registry lists {} ({}). Check for updates again.",
            entry.version, entry.git_ref
        )),
        (Origin::DefaultRegistry, None) => Err(format!(
            "Refusing to update {name}: the default registry doesn't list it any more."
        )),
        (Origin::OtherRegistry, _) => Ok(None),
    }
}

/// What the registry vouches for about `target` (from [`update_target`]),
/// given what checking its signature found: the commit it must resolve to,
/// and whether it's verified. Refused as [`admit`] refuses an install.
pub fn vouch(
    origin: Origin,
    name: &str,
    target: &ListedRelease,
    check: &SignatureCheck,
) -> Result<VouchedRelease, String> {
    Ok(VouchedRelease {
        commit: target.commit.clone(),
        verified: admit(origin, name, &target.git_ref, check)?,
    })
}

/// A short, readable fingerprint of a public key: the first 16 hex digits
/// of the SHA-256 of what it says, in groups of four
/// (`SHA256:1a2b 3c4d 5e6f 7a8b`).
pub fn fingerprint(public_key: &str) -> String {
    let digest = Sha256::digest(public_key.trim().as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let groups: Vec<&str> = (0..hex.len())
        .step_by(4)
        .map(|start| &hex[start..start + 4])
        .collect();
    format!("SHA256:{}", groups.join(" "))
}

/// The warning for a publisher whose key changed since it was pinned.
pub fn key_changed_warning(name: &str, pinned: &str, offered: &str) -> String {
    format!(
        "Refusing to install {name}: its publisher's signing key changed. You trusted {}, and the registry now offers {}. Someone may have taken over the registry or the publisher's account. If the publisher really changed their key, check it with them, then remove the registry in Settings › Sources and add it again to trust the new key.",
        fingerprint(pinned),
        fingerprint(offered)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "e5d4c3b2a1f0e5d4c3b2a1f0e5d4c3b2a1f0e5d4";

    fn release<'a>(commit: Option<&'a str>, signature: Option<&'a str>) -> Release<'a> {
        Release {
            kind: "agent",
            id: "arxiv",
            version: "1.0.0",
            commit,
            signature,
        }
    }

    /// Every release the default registry lists today, as published. The
    /// shipped key must verify them all, or the first-party packs stop
    /// installing.
    #[test]
    fn the_shipped_key_verifies_every_published_first_party_release() {
        for (kind, id, version, commit, signature) in [
            ("pack", "dev-pack", "0.5.1", "b4b17909592052a809a6a4f29911736929a33cd2", "ed25519:pJQLEsskNSjPq0xfotWrdrjA7nVDRC6vjZuydxt1qVGMK4bo10RL7bmOzoLUjl38wvONorVf6Mx/AUnXdxnyCg=="),
            ("pack", "personal-backup", "0.1.1", "36dfcaffd660d93f8a61e5f4cae652e57c474610", "ed25519:LXTZHlQmpOplvhdYbNGOzNXcBrVR2N9nc7Jz4qyXKH9NmKD/TvvcwOOs4CrFfZpb2o2xwWMzv19Gtc4HjNoqAA=="),
            ("pack", "social-pack", "0.1.0", "c46443c39cdec4266dcad6a7980a82505ff7af40", "ed25519:/4sxD8gPNq209pdgXqVt7ZM1M6KLTWv4nDpNO56PQQnTKxg6j9EbhhN0DaX+Z6Q3d+kz9BPSCDgfAtKvF3/PCA=="),
            ("agent", "pr-reviewer", "0.5.1", "b4b17909592052a809a6a4f29911736929a33cd2", "ed25519:OpV7nr0JpA7DAonGowu0SP7kqijnS6AkAd247WcXMdBadO6bFoMGWH85wfDGtaUBpyn8jPkFuI2dCae9RAyyDw=="),
            ("agent", "aws-ops-analyst", "0.5.1", "b4b17909592052a809a6a4f29911736929a33cd2", "ed25519:/QkzdLbbWz714FQCzVdimtvGo/4yFSknHNNPrtNSuQlMozzNm2UbIbxZZC63ICxrGdbTQD5fst3ySuY8R/e2Bg=="),
            ("agent", "backup-keeper", "0.1.1", "36dfcaffd660d93f8a61e5f4cae652e57c474610", "ed25519:xItt6hdf2SMoO9WvO2H8x8AdmWKlcPRhpNH4mHUAQuACmSQxJg3ncN5b19ckuotDqIbV3y0j2ufrglA2gvLBBQ=="),
            ("agent", "social-manager", "0.1.0", "c46443c39cdec4266dcad6a7980a82505ff7af40", "ed25519:zvy8hj3HohyqXSX6T3fEAbedUrwCmcXXYhhbnA9LAvnaadkqMKNEKqIS+OgVal/RC1tC6HwMY2+tsko/HXneDA=="),
        ] {
            let release = Release {
                kind,
                id,
                version,
                commit: Some(commit),
                signature: Some(signature),
            };
            assert_eq!(
                check(&PublisherKey::Shipped, &release),
                SignatureCheck::Verified { pin: None },
                "{kind} {id} {version}"
            );
        }
    }

    #[test]
    fn only_the_first_party_publisher_uses_the_shipped_key() {
        for (publisher, shipped) in [
            ("vaultdbai", true),
            ("VaultDBai", true),
            ("alice", false),
            ("vaultdbai-fan", false),
        ] {
            assert_eq!(
                PublisherKey::shipped(publisher) == Some(PublisherKey::Shipped),
                shipped,
                "{publisher}"
            );
        }
    }

    #[test]
    fn a_registry_key_is_held_against_its_pin() {
        let key = |k: &str| Some(k.to_string());
        for (pinned, fetched, expected) in [
            (
                None,
                Ok(key("ed25519:a")),
                PublisherKey::FirstUse("ed25519:a".into()),
            ),
            (
                Some("ed25519:a"),
                Ok(key("ed25519:a")),
                PublisherKey::Pinned("ed25519:a".into()),
            ),
            (
                Some("ed25519:a"),
                Ok(key("ed25519:b")),
                PublisherKey::Changed {
                    pinned: "ed25519:a".into(),
                    offered: "ed25519:b".into(),
                },
            ),
            (Some("ed25519:a"), Ok(None), PublisherKey::NoKey),
            (
                None,
                Err("timed out".to_string()),
                PublisherKey::Unreachable("timed out".into()),
            ),
        ] {
            assert_eq!(
                PublisherKey::from_registry(pinned, fetched.clone()),
                expected,
                "{pinned:?} {fetched:?}"
            );
        }
    }

    #[test]
    fn a_signature_verifies_only_against_the_trusted_key() {
        let (secret, public) = signature::generate();
        let (_, stranger) = signature::generate();
        let signature = signature::sign(&secret, "agent", "arxiv", "1.0.0", COMMIT).unwrap();
        let signed = release(Some(COMMIT), Some(&signature));
        for (key, release, expected) in [
            (
                PublisherKey::FirstUse(public.clone()),
                signed,
                SignatureCheck::Verified {
                    pin: Some(public.clone()),
                },
            ),
            (
                PublisherKey::Pinned(public.clone()),
                signed,
                SignatureCheck::Verified { pin: None },
            ),
            (
                PublisherKey::Pinned(stranger.clone()),
                signed,
                SignatureCheck::BadSignature,
            ),
            (PublisherKey::Shipped, signed, SignatureCheck::BadSignature),
            (
                PublisherKey::Pinned(public.clone()),
                release(Some(COMMIT), None),
                SignatureCheck::Unsigned,
            ),
            (
                PublisherKey::Pinned(public.clone()),
                release(None, signed.signature),
                SignatureCheck::Unsigned,
            ),
            (PublisherKey::NoKey, signed, SignatureCheck::Unsigned),
            (
                PublisherKey::Changed {
                    pinned: public.clone(),
                    offered: stranger.clone(),
                },
                signed,
                SignatureCheck::KeyChanged {
                    pinned: public.clone(),
                    offered: stranger.clone(),
                },
            ),
            (
                PublisherKey::Unreachable("offline".into()),
                signed,
                SignatureCheck::CouldNotCheck("offline".into()),
            ),
            // An unsigned entry needs no key, so "couldn't fetch it" doesn't matter
            (
                PublisherKey::Unreachable("offline".into()),
                release(None, None),
                SignatureCheck::Unsigned,
            ),
        ] {
            assert_eq!(check(&key, &release), expected, "{key:?}");
        }
    }

    #[test]
    fn only_the_default_registry_is_the_default_origin() {
        for (url, origin) in [
            (DEFAULT_REGISTRY, Origin::DefaultRegistry),
            (
                "https://github.com/vaultdbai/nandi-registry.git",
                Origin::DefaultRegistry,
            ),
            (
                "github.com/vaultdbai/nandi-registry-fork",
                Origin::OtherRegistry,
            ),
            ("file:///tmp/registry", Origin::OtherRegistry),
        ] {
            assert_eq!(Origin::of(url), origin, "{url}");
        }
    }

    fn changed() -> SignatureCheck {
        SignatureCheck::KeyChanged {
            pinned: "ed25519:old".into(),
            offered: "ed25519:new".into(),
        }
    }

    /// `Ok(verified)`, or a word the refusal must contain
    #[test]
    fn the_default_registry_admits_only_signed_release_tags() {
        use Origin::{DefaultRegistry, OtherRegistry};
        let verified = SignatureCheck::Verified { pin: None };
        let offline = SignatureCheck::CouldNotCheck("timed out".into());
        for (origin, git_ref, check, expected) in [
            (DefaultRegistry, "v1.0.0", &verified, Ok(true)),
            (
                DefaultRegistry,
                "v1.0.0",
                &SignatureCheck::Unsigned,
                Err("isn't signed"),
            ),
            (
                DefaultRegistry,
                "v1.0.0",
                &SignatureCheck::BadSignature,
                Err("doesn't match"),
            ),
            (DefaultRegistry, "main", &verified, Err("release tag")),
            (DefaultRegistry, "v1.0.0", &offline, Err("Try again later")),
            (DefaultRegistry, "v1.0.0", &changed(), Err("key changed")),
            (OtherRegistry, "v1.0.0", &verified, Ok(true)),
            (OtherRegistry, "main", &SignatureCheck::Unsigned, Ok(false)),
            (
                OtherRegistry,
                "v1.0.0",
                &SignatureCheck::BadSignature,
                Ok(false),
            ),
            (OtherRegistry, "v1.0.0", &offline, Err("Try again later")),
            (OtherRegistry, "v1.0.0", &changed(), Err("key changed")),
        ] {
            let admitted = admit(origin, "ArXiv", git_ref, check);
            match expected {
                Ok(verified) => {
                    assert_eq!(admitted, Ok(verified), "{origin:?} {git_ref} {check:?}")
                }
                Err(word) => {
                    let refused = admitted.unwrap_err();
                    assert!(
                        refused.contains(word),
                        "{origin:?} {git_ref} {check:?}: {refused}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_default_registry_update_is_exactly_the_listed_release() {
        use Origin::{DefaultRegistry, OtherRegistry};
        let listed = ListedRelease {
            version: "1.1.0".into(),
            git_ref: "v1.1.0".into(),
            commit: Some(COMMIT.into()),
        };
        // `Ok(true)`: the listed release is the target; `Ok(false)`: nothing
        // vouches for it; `Err(word)`: refused
        for (origin, new_ref, entry, expected) in [
            (DefaultRegistry, "v1.1.0", Some(&listed), Ok(true)),
            (DefaultRegistry, "v9.0.0", Some(&listed), Err("lists 1.1.0")),
            (DefaultRegistry, "v1.1.0", None, Err("doesn't list it")),
            (OtherRegistry, "v1.1.0", Some(&listed), Ok(true)),
            (OtherRegistry, "v9.0.0", Some(&listed), Ok(false)),
            (OtherRegistry, "v1.1.0", None, Ok(false)),
        ] {
            let target = update_target(origin, "ArXiv", new_ref, entry);
            match expected {
                Ok(listed_target) => assert_eq!(
                    target.unwrap().is_some(),
                    listed_target,
                    "{origin:?} {new_ref}"
                ),
                Err(word) => {
                    let refused = target.unwrap_err();
                    assert!(refused.contains(word), "{origin:?} {new_ref}: {refused}");
                }
            }
        }
    }

    #[test]
    fn an_update_is_vouched_for_by_its_signed_entry() {
        use Origin::{DefaultRegistry, OtherRegistry};
        let listed = ListedRelease {
            version: "1.1.0".into(),
            git_ref: "v1.1.0".into(),
            commit: Some(COMMIT.into()),
        };
        let verified = SignatureCheck::Verified { pin: None };
        let pinned = |verified| {
            Ok(VouchedRelease {
                commit: Some(COMMIT.into()),
                verified,
            })
        };
        assert_eq!(
            vouch(DefaultRegistry, "ArXiv", &listed, &verified),
            pinned(true)
        );
        assert_eq!(
            vouch(OtherRegistry, "ArXiv", &listed, &SignatureCheck::Unsigned),
            pinned(false)
        );
        assert!(
            vouch(DefaultRegistry, "ArXiv", &listed, &SignatureCheck::Unsigned)
                .unwrap_err()
                .contains("isn't signed")
        );
        assert!(vouch(OtherRegistry, "ArXiv", &listed, &changed())
            .unwrap_err()
            .contains("key changed"));
    }

    #[test]
    fn a_changed_key_warning_names_both_fingerprints() {
        let (_, old) = signature::generate();
        let (_, new) = signature::generate();
        let warning = key_changed_warning("ArXiv", &old, &new);
        assert!(warning.contains(&fingerprint(&old)), "{warning}");
        assert!(warning.contains(&fingerprint(&new)), "{warning}");
        assert_ne!(fingerprint(&old), fingerprint(&new));
        assert_eq!(
            fingerprint(&old),
            fingerprint(&format!(" {old}\n")),
            "whitespace doesn't count"
        );
        let shape = fingerprint(FIRST_PARTY_PUBLIC_KEY);
        assert!(
            shape.starts_with("SHA256:") && shape.len() == "SHA256:".len() + 19,
            "{shape}"
        );
    }
}
