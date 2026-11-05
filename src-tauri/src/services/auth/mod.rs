//! Who this Nandi instance is: a display name and a user slug, kept in the
//! frontmatter of `~/.nandi/repo/NANDI.md` (the gateway manifest, see
//! `services::gateway`). Identity is local, and sharing goes through an
//! optional git remote (`services::remote`) and the relay. The only sign-in
//! is the optional GitHub one (`github`), for github.com repositories.
//!
//! The slug names the only branch Nandi ever pushes: `<prefix>/<slug>`
//! (`nandi/alice-chen` by default).
//!
//! Writes touch only the identity keys of the frontmatter (`display_name`,
//! `owner.display_name`, `user_slug`, and `name` while it's still the
//! default `<name>'s Nandi`); every other key and the Markdown body are kept
//! as they are. The file is replaced atomically (temp file + rename).

pub mod github;
pub mod oauth;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use specta::Type;

use crate::services::domain::agent::split_frontmatter;
use crate::services::events::{self as bus_events, EventBus, NandiEvent};
use crate::services::gateway::manifest::{self, with_greeting, DEFAULT_TEMPLATE, MANIFEST_FILE};
use crate::services::remote::{self, credentials, CredentialKind, DEFAULT_BRANCH_PREFIX};
use crate::services::repos::fs_atomic::write_atomic;
use crate::services::AppState;
use crate::utils::nandi_config::{self, NandiConfig};

/// `identity://changed`
pub const IDENTITY_CHANGED: &str = "identity://changed";

/// Display names are 1–64 characters (after trimming)
const MAX_NAME_CHARS: usize = 64;
/// Slugs are 2–32 characters
const MAX_SLUG_CHARS: usize = 32;

/// One NANDI.md read-modify-write at a time
static MANIFEST_LOCK: Mutex<()> = Mutex::new(());

/// The local identity, as `NANDI.md` holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStatus {
    /// `NANDI.md` exists and names someone; `false` routes to onboarding
    pub onboarded: bool,
    pub display_name: Option<String>,
    /// `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$`
    pub user_slug: Option<String>,
    /// `<prefix>/<slug>`, the branch pushes go to
    pub branch: Option<String>,
    /// Where first-run setup stopped on this Nandi home; `None` before it
    /// started
    #[serde(default)]
    pub onboarding_step: Option<OnboardingStep>,
}

/// The first-run step to resume at. Machine-local runtime state in
/// `local_settings` (per Nandi home), not the gold copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OnboardingStep {
    Name,
    Model,
    /// Connect the mailboxes the Chief of Staff reads (optional)
    Email,
    Network,
    Packs,
    Done,
}

/// `local_settings` key for [`OnboardingStep`]
pub const ONBOARDING_STEP_KEY: &str = "onboarding.step";

/// `identity://changed`, carrying the new status
#[derive(Debug, Clone, Serialize)]
pub struct IdentityChanged {
    #[serde(flatten)]
    pub status: IdentityStatus,
}

impl NandiEvent for IdentityChanged {
    const NAME: &'static str = IDENTITY_CHANGED;
}

// ---------------------------------------------------------------------------
// Service functions
// ---------------------------------------------------------------------------

/// `~/.nandi/repo/NANDI.md`
pub(crate) fn manifest_path(state: &AppState) -> PathBuf {
    manifest::manifest_path(&state.store().layout().root)
}

fn load_config(state: &AppState) -> NandiConfig {
    nandi_config::load_from(state.nandi_dir()).unwrap_or_else(|e| {
        log::warn!("Reading the identity without config.json: {e}");
        NandiConfig::default()
    })
}

fn branch_prefix(config: &NandiConfig) -> String {
    config
        .remote
        .as_ref()
        .map(|r| r.branch_prefix.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| DEFAULT_BRANCH_PREFIX.to_string())
}

pub async fn get_identity_status(state: &AppState) -> Result<IdentityStatus, String> {
    let path = manifest_path(state);
    let config = load_config(state);
    let mut status = tokio::task::spawn_blocking(move || status_from(&path, &config))
        .await
        .map_err(|e| format!("Identity task failed: {e}"))??;
    status.onboarding_step = onboarding_step(state).await?;
    Ok(status)
}

async fn onboarding_step(state: &AppState) -> Result<Option<OnboardingStep>, String> {
    let value = state
        .db()
        .call(|conn| crate::db::get_setting(conn, ONBOARDING_STEP_KEY))
        .await?;
    // An unreadable value counts as unset rather than trapping onboarding
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// Remembers where first-run setup is, so a relaunch resumes there
pub async fn set_onboarding_step(state: &AppState, step: OnboardingStep) -> Result<(), String> {
    let value = serde_json::to_string(&step).map_err(|e| e.to_string())?;
    state
        .db()
        .call(move |conn| {
            conn.execute(
                "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![ONBOARDING_STEP_KEY, value],
            )
            .map(|_| ())
        })
        .await?;
    emit_changed(state).await;
    Ok(())
}

fn status_from(path: &Path, config: &NandiConfig) -> Result<IdentityStatus, String> {
    let identity = read_identity(path)?;
    let prefix = branch_prefix(config);
    Ok(IdentityStatus {
        onboarded: identity.exists && identity.display_name.is_some(),
        branch: identity
            .user_slug
            .as_ref()
            .map(|slug| format!("{prefix}/{slug}")),
        display_name: identity.display_name,
        user_slug: identity.user_slug,
        onboarding_step: None,
    })
}

/// Writes `display_name` to `NANDI.md` (creating it from the default
/// template if missing); derives the slug when none is set.
pub async fn set_display_name(state: &AppState, name: String) -> Result<(), String> {
    let name = validate_display_name(&name)?;
    let path = manifest_path(state);
    let prefix = branch_prefix(&load_config(state));
    let slug = tokio::task::spawn_blocking(move || write_display_name(&path, &name, &prefix))
        .await
        .map_err(|e| format!("Identity task failed: {e}"))??;
    sync_remote_slug(state, &slug)?;
    emit_changed(state).await;
    Ok(())
}

pub async fn set_user_slug(state: &AppState, slug: String) -> Result<(), String> {
    let slug = slug.trim().to_string();
    let prefix = branch_prefix(&load_config(state));
    validate_slug_for(&slug, &prefix)?;
    let path = manifest_path(state);
    let written = slug.clone();
    tokio::task::spawn_blocking(move || write_user_slug(&path, &written))
        .await
        .map_err(|e| format!("Identity task failed: {e}"))??;
    sync_remote_slug(state, &slug)?;
    emit_changed(state).await;
    Ok(())
}

/// The remote pushes to `<prefix>/<slug>`: a configured remote follows
/// the identity's slug
fn sync_remote_slug(state: &AppState, slug: &str) -> Result<(), String> {
    let config = load_config(state);
    if config
        .remote
        .as_ref()
        .is_some_and(|remote| remote.user_slug != slug)
    {
        nandi_config::update_in(state.nandi_dir(), |config| {
            if let Some(remote) = config.remote.as_mut() {
                remote.user_slug = slug.to_string();
            }
        })?;
    }
    Ok(())
}

async fn emit_changed(state: &AppState) {
    emit_changed_at(state.bus(), &state.store().layout().root, state.nandi_dir());
}

/// Emits `identity://changed` with the status read from `repo_root` and
/// the config in `nandi_dir`
pub(crate) fn emit_changed_at(bus: &EventBus, repo_root: &Path, nandi_dir: &Path) {
    let config = nandi_config::load_from(nandi_dir).unwrap_or_default();
    match status_from(&manifest::manifest_path(repo_root), &config) {
        Ok(status) => bus_events::emit(bus, &IdentityChanged { status }),
        Err(e) => log::warn!("Identity changed but can't be read back: {e}"),
    }
}

/// Writes the user slug to `<repo root>/NANDI.md` (validated by the caller)
pub(crate) fn write_slug_at(repo_root: &Path, slug: &str) -> Result<(), String> {
    validate_slug(slug)?;
    write_user_slug(&manifest::manifest_path(repo_root), slug)
}

/// The user slug in `<repo root>/NANDI.md`, if one is set (for services
/// that only hold a layout, like teams)
pub(crate) fn user_slug_at(repo_root: &Path) -> Option<String> {
    read_identity(&manifest::manifest_path(repo_root))
        .ok()
        .and_then(|identity| identity.user_slug)
}

/// The display name in `<repo root>/NANDI.md`, if one is set
pub(crate) fn display_name_at(repo_root: &Path) -> Option<String> {
    read_identity(&manifest::manifest_path(repo_root))
        .ok()
        .and_then(|identity| identity.display_name)
}

/// A token for GitHub's API (marketplace publishing): the GitHub sign-in,
/// else the remote credential when it is a token. Crate-internal only;
/// never sent to clients.
pub(crate) fn require_token(state: &AppState) -> Result<String, String> {
    if let Some(token) = github::token(state.secrets()) {
        return Ok(token);
    }
    credentials::load(state.secrets())?
        .filter(|c| c.kind == CredentialKind::Token)
        .and_then(|c| c.token)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            "Publishing to GitHub needs you to sign in to GitHub (Settings › Account)".to_string()
        })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// The trimmed name, 1–64 characters
fn validate_display_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let count = name.chars().count();
    if count == 0 {
        return Err("Enter your name".to_string());
    }
    if count > MAX_NAME_CHARS {
        return Err(format!(
            "Your name can be at most {MAX_NAME_CHARS} characters"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("Your name can't contain control characters".to_string());
    }
    Ok(name.to_string())
}

/// `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$`
pub fn validate_slug(slug: &str) -> Result<(), String> {
    let bytes = slug.as_bytes();
    let alnum = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let valid = (2..=MAX_SLUG_CHARS).contains(&bytes.len())
        && alnum(&bytes[0])
        && alnum(&bytes[bytes.len() - 1])
        && bytes.iter().all(|b| alnum(b) || *b == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "'{slug}' isn't a valid user slug: use 2–32 lowercase letters, digits, and hyphens, starting and ending with a letter or digit"
        ))
    }
}

/// A valid slug whose branch `<prefix>/<slug>` may be pushed to
fn validate_slug_for(slug: &str, prefix: &str) -> Result<(), String> {
    validate_slug(slug)?;
    remote::assert_pushable_branch(&format!("{prefix}/{slug}"), prefix)
}

/// A slug from a display name: lowercase ASCII (common accented letters
/// transliterated, other characters dropped), runs of anything else as
/// one hyphen, trimmed, at most 32 characters. May be empty or invalid
/// (a name with no Latin letters); see [`slug_or_fallback`].
pub fn derive_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_hyphen = false;
    for c in name.chars() {
        let mapped = transliterate(c);
        if mapped.is_empty() && !c.is_ascii() && c.is_alphanumeric() {
            // A letter with no ASCII form: dropped, not a separator
            continue;
        }
        let mapped = if mapped.is_empty() {
            c.to_ascii_lowercase().to_string()
        } else {
            mapped.to_string()
        };
        for m in mapped.chars() {
            if m.is_ascii_alphanumeric() {
                if pending_hyphen && !slug.is_empty() {
                    slug.push('-');
                }
                pending_hyphen = false;
                slug.push(m.to_ascii_lowercase());
            } else {
                pending_hyphen = true;
            }
        }
    }
    if slug.len() > MAX_SLUG_CHARS {
        slug.truncate(MAX_SLUG_CHARS);
    }
    slug.trim_matches('-').to_string()
}

/// [`derive_slug`], or `user-<random>` when that isn't a valid slug
fn slug_or_fallback(name: &str) -> String {
    let slug = derive_slug(name);
    if validate_slug(&slug).is_ok() {
        return slug;
    }
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("user-{}", &random[..6])
}

/// ASCII for common Latin letters with diacritics; `""` for anything else
fn transliterate(c: char) -> &'static str {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å'
        | 'Ā' | 'Ă' | 'Ą' => "a",
        'æ' | 'Æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => "c",
        'ď' | 'đ' | 'ð' | 'Ď' | 'Đ' | 'Ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ'
        | 'Ė' | 'Ę' | 'Ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => "g",
        'ĥ' | 'ħ' | 'Ĥ' | 'Ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī'
        | 'Ĭ' | 'Į' | 'İ' => "i",
        'ĵ' | 'Ĵ' => "j",
        'ķ' | 'Ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø'
        | 'Ō' | 'Ŏ' | 'Ő' => "o",
        'œ' | 'Œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' | 'Ŕ' | 'Ŗ' | 'Ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'Ś' | 'Ŝ' | 'Ş' | 'Š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'Ţ' | 'Ť' | 'Ŧ' => "t",
        'þ' | 'Þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ'
        | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => "u",
        'ŵ' | 'Ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ÿ' | 'Ŷ' => "y",
        'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "z",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// NANDI.md
// ---------------------------------------------------------------------------

/// The identity keys of a NANDI.md
#[derive(Debug, Default, PartialEq, Eq)]
struct Identity {
    exists: bool,
    display_name: Option<String>,
    user_slug: Option<String>,
}

/// NANDI.md split into its frontmatter and (untouched) body
struct Manifest {
    frontmatter: Mapping,
    body: String,
}

fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

fn string_at(mapping: &Mapping, name: &str) -> Option<String> {
    mapping
        .get(key(name))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_identity(path: &Path) -> Result<Identity, String> {
    let Some(manifest) = read_manifest(path)? else {
        return Ok(Identity::default());
    };
    let fm = &manifest.frontmatter;
    let owner_name = fm
        .get(key("owner"))
        .and_then(Value::as_mapping)
        .and_then(|owner| string_at(owner, "display_name"));
    Ok(Identity {
        exists: true,
        display_name: string_at(fm, "display_name").or(owner_name),
        user_slug: string_at(fm, "user_slug").filter(|slug| validate_slug(slug).is_ok()),
    })
}

/// `None` if the file doesn't exist; an error if its frontmatter isn't a
/// YAML mapping (never overwritten then)
fn read_manifest(path: &Path) -> Result<Option<Manifest>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read {MANIFEST_FILE}: {e}")),
    };
    parse_manifest(&text).map(Some)
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let (yaml, body) =
        split_frontmatter(text).map_err(|e| format!("{MANIFEST_FILE} is malformed: {e}"))?;
    let frontmatter = match yaml.map(str::trim) {
        None | Some("") => Mapping::new(),
        Some(yaml) => match serde_yaml::from_str::<Value>(yaml) {
            Ok(Value::Mapping(mapping)) => mapping,
            Ok(Value::Null) => Mapping::new(),
            Ok(_) => return Err(format!("{MANIFEST_FILE}'s frontmatter isn't a mapping")),
            Err(e) => return Err(format!("{MANIFEST_FILE} has invalid frontmatter: {e}")),
        },
    };
    Ok(Manifest {
        frontmatter,
        body: body.to_string(),
    })
}

/// The default NANDI.md for `owner`. `{{owner}}` is filled into the body as
/// text, and into the frontmatter through YAML (so any name is safe).
fn default_manifest(owner: &str) -> Result<Manifest, String> {
    let mut manifest = parse_manifest(&DEFAULT_TEMPLATE.replace("{{owner}}", ""))?;
    let (_, body) = split_frontmatter(DEFAULT_TEMPLATE)?;
    manifest.body = body.replace("{{owner}}", owner);
    Ok(manifest)
}

fn render_manifest(manifest: &Manifest) -> Result<String, String> {
    let yaml = serde_yaml::to_string(&Value::Mapping(manifest.frontmatter.clone()))
        .map_err(|e| format!("Failed to write {MANIFEST_FILE}: {e}"))?;
    Ok(format!("---\n{yaml}---\n{}", manifest.body))
}

/// Reads NANDI.md (or the default for a missing one), applies `change`,
/// and replaces the file atomically. Returns the change's result.
fn update_manifest<T>(
    path: &Path,
    owner: &str,
    change: impl FnOnce(&mut Manifest) -> T,
) -> Result<T, String> {
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut manifest = match read_manifest(path)? {
        Some(manifest) => manifest,
        None => default_manifest(owner)?,
    };
    let result = change(&mut manifest);
    let text = render_manifest(&manifest)?;
    write_atomic(path, text.as_bytes())
        .map_err(|e| format!("Failed to write {MANIFEST_FILE}: {e}"))?;
    Ok(result)
}

/// Sets the name (and `name` while it's still the default); derives a
/// slug when none is set. Returns the slug.
fn write_display_name(path: &Path, name: &str, prefix: &str) -> Result<String, String> {
    update_manifest(path, name, |manifest| {
        let fm = &mut manifest.frontmatter;
        let old = string_at(fm, "display_name").unwrap_or_default();
        // The template's greeting follows the name ("I'm ." before onboarding)
        manifest.body = with_greeting(&manifest.body, &old, name);
        let title = fm.get(key("name")).and_then(Value::as_str).map(str::trim);
        let is_default_title = match title {
            None | Some("") => true,
            Some(title) => {
                title == format!("{old}'s Nandi") || title == "'s Nandi" || title == "My Nandi"
            }
        };
        if is_default_title {
            fm.insert(key("name"), Value::String(format!("{name}'s Nandi")));
        }
        fm.insert(key("display_name"), Value::String(name.to_string()));
        if !fm.get(key("owner")).is_some_and(Value::is_mapping) {
            fm.insert(key("owner"), Value::Mapping(Mapping::new()));
        }
        if let Some(owner) = fm.get_mut(key("owner")).and_then(Value::as_mapping_mut) {
            owner.insert(key("display_name"), Value::String(name.to_string()));
        }
        match string_at(fm, "user_slug").filter(|slug| validate_slug_for(slug, prefix).is_ok()) {
            Some(slug) => slug,
            None => {
                let slug = slug_or_fallback(name);
                fm.insert(key("user_slug"), Value::String(slug.clone()));
                slug
            }
        }
    })
}

fn write_user_slug(path: &Path, slug: &str) -> Result<(), String> {
    update_manifest(path, "", |manifest| {
        manifest
            .frontmatter
            .insert(key("user_slug"), Value::String(slug.to_string()));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_slugs_from_names() {
        assert_eq!(derive_slug("Alice Chen"), "alice-chen");
        assert_eq!(derive_slug("  José  Ñ!!"), "jose-n");
        assert_eq!(derive_slug("a -- b __ c"), "a-b-c");
        assert_eq!(derive_slug("--Mona--"), "mona");
        assert_eq!(derive_slug("Zoë O'Brien-Łukasz"), "zoe-o-brien-lukasz");
        assert_eq!(derive_slug("Straße"), "strasse");
        assert_eq!(derive_slug("李雷"), "");
        assert_eq!(derive_slug("Ann 李"), "ann");
        let long = derive_slug("abcdefghij abcdefghij abcdefghij abcdefghij");
        assert!(long.len() <= 32);
        assert!(validate_slug(&long).is_ok(), "{long}");
        // Truncation never leaves a trailing hyphen
        let cut = derive_slug(&format!("{} b", "a".repeat(31)));
        assert_eq!(cut, "a".repeat(31));
    }

    #[test]
    fn falls_back_when_no_slug_can_be_derived() {
        let slug = slug_or_fallback("李雷");
        assert!(slug.starts_with("user-"));
        assert!(validate_slug(&slug).is_ok());
        assert!(slug_or_fallback("X").starts_with("user-"));
    }

    #[test]
    fn validates_slugs() {
        for ok in ["ab", "alice", "alice-chen", "a1", "0-9", &"a".repeat(32)] {
            assert!(validate_slug(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "a",
            "-alice",
            "alice-",
            "Alice",
            "alice chen",
            "alice_chen",
            "alicé",
            "alice/chen",
            &"a".repeat(33),
        ] {
            assert!(validate_slug(bad).is_err(), "{bad}");
        }
        assert!(validate_slug_for("alice", "nandi").is_ok());
        assert!(validate_slug_for("alice", "main").is_err());
    }

    #[test]
    fn validates_display_names() {
        assert_eq!(validate_display_name("  Mona  ").unwrap(), "Mona");
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name(&"x".repeat(64)).is_ok());
        assert!(validate_display_name(&"x".repeat(65)).is_err());
        assert!(validate_display_name("a\nb").is_err());
    }

    #[test]
    fn creates_nandi_md_from_the_template() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("repo").join(MANIFEST_FILE);
        assert_eq!(read_identity(&path).unwrap(), Identity::default());

        let slug = write_display_name(&path, "Alice \"Al\" Chen", "nandi").unwrap();
        assert_eq!(slug, "alice-al-chen");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("I'm Alice \"Al\" Chen."));
        let manifest = parse_manifest(&text).unwrap();
        let fm = &manifest.frontmatter;
        assert_eq!(string_at(fm, "name").unwrap(), "Alice \"Al\" Chen's Nandi");
        assert_eq!(string_at(fm, "description").unwrap(), "My Nandi agents.");
        let policy = fm.get(key("policy")).unwrap().as_mapping().unwrap();
        assert_eq!(policy.get(key("max_concurrent")).unwrap().as_u64(), Some(3));
        let identity = read_identity(&path).unwrap();
        assert!(identity.exists);
        assert_eq!(identity.display_name.as_deref(), Some("Alice \"Al\" Chen"));
        assert_eq!(identity.user_slug.as_deref(), Some("alice-al-chen"));
        // No temp files left behind
        let entries = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1);
    }

    #[test]
    fn onboarding_fills_in_the_template_name_and_title() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MANIFEST_FILE);
        // As first launch writes it, before onboarding knows the name
        std::fs::write(
            &path,
            crate::services::gateway::manifest::render_default(""),
        )
        .unwrap();
        write_display_name(&path, "Alice", "nandi").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let manifest = parse_manifest(&text).unwrap();
        assert!(
            manifest.body.contains(
                "
I'm Alice.
"
            ),
            "{}",
            manifest.body
        );
        assert!(!manifest.body.contains("I'm ."));
        assert_eq!(
            string_at(&manifest.frontmatter, "name").unwrap(),
            "Alice's Nandi"
        );
    }

    #[test]
    fn set_display_name_preserves_the_rest_of_nandi_md() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MANIFEST_FILE);
        let body = "\n# About me\n\nI do *research*.\n\n---\n\nTrailing rule above.\n";
        std::fs::write(
            &path,
            format!(
                "---\nname: Research Hub\ndisplay_name: Old\nowner:\n  display_name: Old\n  github_handle: old\nuser_slug: old-slug\npolicy:\n  max_concurrent: 7\ncustom: [1, 2]\n---\n{body}"
            ),
        )
        .unwrap();

        let slug = write_display_name(&path, "New Name", "nandi").unwrap();
        // An existing slug is kept
        assert_eq!(slug, "old-slug");
        let text = std::fs::read_to_string(&path).unwrap();
        let manifest = parse_manifest(&text).unwrap();
        assert_eq!(manifest.body, body);
        let fm = &manifest.frontmatter;
        // A custom title is not the default: unchanged
        assert_eq!(string_at(fm, "name").unwrap(), "Research Hub");
        assert_eq!(string_at(fm, "display_name").unwrap(), "New Name");
        let owner = fm.get(key("owner")).unwrap().as_mapping().unwrap();
        assert_eq!(string_at(owner, "display_name").unwrap(), "New Name");
        assert_eq!(string_at(owner, "github_handle").unwrap(), "old");
        assert_eq!(string_at(fm, "user_slug").unwrap(), "old-slug");
        let policy = fm.get(key("policy")).unwrap().as_mapping().unwrap();
        assert_eq!(policy.get(key("max_concurrent")).unwrap().as_u64(), Some(7));
        assert!(fm.get(key("custom")).unwrap().is_sequence());
        // Keys keep their order
        let keys: Vec<&str> = fm.keys().filter_map(Value::as_str).collect();
        assert_eq!(
            keys,
            [
                "name",
                "display_name",
                "owner",
                "user_slug",
                "policy",
                "custom"
            ]
        );
    }

    #[test]
    fn the_default_title_follows_the_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MANIFEST_FILE);
        std::fs::write(&path, DEFAULT_TEMPLATE.replace("{{owner}}", "octocat")).unwrap();
        write_display_name(&path, "Mona Lisa", "nandi").unwrap();
        let fm = parse_manifest(&std::fs::read_to_string(&path).unwrap())
            .unwrap()
            .frontmatter;
        assert_eq!(string_at(&fm, "name").unwrap(), "Mona Lisa's Nandi");
        assert_eq!(string_at(&fm, "user_slug").unwrap(), "mona-lisa");
    }

    #[test]
    fn set_user_slug_writes_and_invalid_frontmatter_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MANIFEST_FILE);
        write_display_name(&path, "Alice", "nandi").unwrap();
        write_user_slug(&path, "alice-2").unwrap();
        assert_eq!(
            read_identity(&path).unwrap().user_slug.as_deref(),
            Some("alice-2")
        );

        let broken = "---\nname: [unclosed\n---\nbody\n";
        std::fs::write(&path, broken).unwrap();
        assert!(read_identity(&path).is_err());
        assert!(write_display_name(&path, "Bob", "nandi").is_err());
        assert!(write_user_slug(&path, "bob").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn a_nandi_md_without_frontmatter_keeps_its_text_as_the_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MANIFEST_FILE);
        std::fs::write(&path, "Just notes.\n").unwrap();
        let identity = read_identity(&path).unwrap();
        assert!(identity.exists && identity.display_name.is_none());
        write_display_name(&path, "Ann", "nandi").unwrap();
        let manifest = parse_manifest(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(manifest.body, "Just notes.\n");
        assert_eq!(
            string_at(&manifest.frontmatter, "user_slug").unwrap(),
            "ann"
        );
    }

    #[tokio::test]
    async fn identity_status_tracks_writes_and_emits() {
        use std::sync::{Arc, Mutex as StdMutex};

        #[derive(Default)]
        struct Sink(StdMutex<Vec<(&'static str, serde_json::Value)>>);
        impl crate::services::events::EventSink for Sink {
            fn emit_json(&self, name: &'static str, payload: serde_json::Value) {
                self.0.lock().unwrap().push((name, payload));
            }
        }
        let sink = Arc::new(Sink::default());
        let state = AppState::for_tests(vec![sink.clone()]);
        let _ = std::fs::remove_file(manifest_path(&state));

        let fresh = get_identity_status(&state).await.unwrap();
        assert!(!fresh.onboarded);
        assert_eq!(fresh.branch, None);

        assert!(set_display_name(&state, "   ".into()).await.is_err());
        set_display_name(&state, "Alice Chen".into()).await.unwrap();
        let status = get_identity_status(&state).await.unwrap();
        assert!(status.onboarded);
        assert_eq!(status.display_name.as_deref(), Some("Alice Chen"));
        assert_eq!(status.user_slug.as_deref(), Some("alice-chen"));
        assert_eq!(status.branch.as_deref(), Some("nandi/alice-chen"));

        assert!(set_user_slug(&state, "Not Valid".into()).await.is_err());
        set_user_slug(&state, "alice".into()).await.unwrap();
        let status = get_identity_status(&state).await.unwrap();
        assert_eq!(status.branch.as_deref(), Some("nandi/alice"));

        let events = sink.0.lock().unwrap();
        let changed: Vec<_> = events
            .iter()
            .filter(|(name, _)| *name == IDENTITY_CHANGED)
            .collect();
        assert_eq!(changed.len(), 2);
        assert_eq!(changed[1].1["userSlug"], "alice");
        assert_eq!(changed[1].1["onboarded"], true);
    }

    #[tokio::test]
    async fn onboarding_step_is_remembered_per_home() {
        let state = AppState::for_tests(vec![]);
        assert_eq!(
            get_identity_status(&state).await.unwrap().onboarding_step,
            None
        );
        for step in [
            OnboardingStep::Model,
            OnboardingStep::Email,
            OnboardingStep::Network,
            OnboardingStep::Done,
        ] {
            set_onboarding_step(&state, step).await.unwrap();
            assert_eq!(
                get_identity_status(&state).await.unwrap().onboarding_step,
                Some(step)
            );
        }
        // A value a newer version wrote reads as unset, not an error
        state
            .db()
            .call(|conn| {
                crate::db::set_setting(conn, ONBOARDING_STEP_KEY, &serde_json::json!("later"))
            })
            .await
            .unwrap();
        assert_eq!(
            get_identity_status(&state).await.unwrap().onboarding_step,
            None
        );
    }
}
