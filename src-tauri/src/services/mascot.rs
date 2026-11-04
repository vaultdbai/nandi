//! The mascot: the
//! bull's name, which the user may change, the lines it speaks in its
//! voice (third person, calm, no exclamation marks), and the hidden
//! phrase. The phrase only ever produces a line of text: it can't
//! approve, run or route agent work.
//!
//! The name lives in `~/.nandi/config.json` (`mascot_name`), so the app and
//! the CLI share it.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::utils::nandi_config;

/// The bull's name until the user picks another
pub const DEFAULT_NAME: &str = "Nandi";
/// Longest name, in characters
pub const MAX_NAME_CHARS: usize = 24;
/// `config.json` key
const CONFIG_KEY: &str = "mascot_name";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MascotInfo {
    /// What the bull is called now
    pub name: String,
    /// Still the default
    pub is_default: bool,
}

/// A name the bull can go by: 1–24 letters, digits, spaces, `-`, `'` or
/// `.`, starting with a letter or digit. Returns it trimmed.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = name.chars().count();
    if count == 0 {
        return Err("Give the bull a name".to_string());
    }
    if count > MAX_NAME_CHARS {
        return Err(format!(
            "Keep the name to {MAX_NAME_CHARS} characters or fewer"
        ));
    }
    if !name.chars().next().is_some_and(char::is_alphanumeric) {
        return Err("Start the name with a letter or a digit".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.'))
    {
        return Err("Use letters, digits, spaces, hyphens, apostrophes or dots".to_string());
    }
    Ok(name)
}

/// The bull's name in the config in `dir` (the default if none or invalid)
pub fn read(dir: &Path) -> MascotInfo {
    let saved = nandi_config::load_from(dir)
        .ok()
        .and_then(|config| config.extra.get(CONFIG_KEY).cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .and_then(|name| validate_name(&name).ok());
    match saved {
        Some(name) => MascotInfo {
            is_default: name == DEFAULT_NAME,
            name,
        },
        None => MascotInfo {
            name: DEFAULT_NAME.to_string(),
            is_default: true,
        },
    }
}

/// Renames the bull (`None`: back to the default)
pub fn write(dir: &Path, name: Option<&str>) -> Result<MascotInfo, String> {
    let name = name.map(validate_name).transpose()?;
    nandi_config::update_in(dir, |config| match &name {
        Some(name) if name != DEFAULT_NAME => {
            config.extra.insert(
                CONFIG_KEY.to_string(),
                serde_json::Value::String(name.clone()),
            );
        }
        _ => {
            config.extra.remove(CONFIG_KEY);
        }
    })?;
    Ok(read(dir))
}

// ---------------------------------------------------------------------------
// Lines, in the bull's voice
// ---------------------------------------------------------------------------

/// "Nandi is up. 3 agents ready."
pub fn line_up(name: &str, agents: usize) -> String {
    match agents {
        1 => format!("{name} is up. 1 agent ready."),
        n => format!("{name} is up. {n} agents ready."),
    }
}

/// "Nandi is now called Gauri."
pub fn line_renamed(name: &str) -> String {
    format!("{DEFAULT_NAME} is now called {name}.")
}

// ---------------------------------------------------------------------------
// The hidden phrase
// ---------------------------------------------------------------------------

/// Lowercase words, punctuation dropped, single spaces
fn normalize(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 64-bit FNV-1a, so the phrase isn't spelled out in the binary
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The phrase's two spellings (the middle vowel differs), hashed
const PHRASES: [u64; 2] = [0xed82_09df_e137_cbab, 0x1bb3_2edf_e88b_a2a3];

/// The nickname, assembled only when the phrase is found
fn nickname() -> String {
    [65u8, 112, 112, 97]
        .iter()
        .map(|b| char::from(*b))
        .collect()
}

/// What the phrase gets: a line, nothing more
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhraseResponse {
    pub line: String,
}

/// The reply to the hidden phrase, or `None` for anything else. Pure:
/// it reads nothing and changes nothing.
pub fn handle_phrase(input: &str, name: &str) -> Option<PhraseResponse> {
    let hash = fnv1a(&normalize(input));
    PHRASES.contains(&hash).then(|| PhraseResponse {
        line: format!("{} rises, steady as ever. {name} carries on.", nickname()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked_and_trimmed() {
        assert_eq!(validate_name("  Gauri  ").unwrap(), "Gauri");
        assert_eq!(validate_name("Big   Bull").unwrap(), "Big Bull");
        assert!(validate_name("").is_err());
        assert!(validate_name("-dash").is_err());
        assert!(validate_name("<script>").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert_eq!(validate_name("Nandī").unwrap(), "Nandī");
    }

    #[test]
    fn the_name_is_saved_and_reset() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).name, DEFAULT_NAME);
        let renamed = write(dir.path(), Some("Gauri")).unwrap();
        assert_eq!(
            renamed,
            MascotInfo {
                name: "Gauri".into(),
                is_default: false
            }
        );
        assert_eq!(read(dir.path()).name, "Gauri");
        assert!(write(dir.path(), Some("!!")).is_err());
        assert_eq!(read(dir.path()).name, "Gauri");
        assert!(write(dir.path(), None).unwrap().is_default);
    }

    #[test]
    fn lines_follow_the_voice() {
        assert_eq!(line_up("Gauri", 3), "Gauri is up. 3 agents ready.");
        assert_eq!(line_up("Gauri", 1), "Gauri is up. 1 agent ready.");
        assert_eq!(line_renamed("Gauri"), "Nandi is now called Gauri.");
        for line in [line_up("N", 2), line_renamed("G")] {
            assert!(!line.contains('!'), "{line}");
        }
    }

    #[test]
    fn the_phrase_matches_both_spellings_in_any_case() {
        assert_eq!(fnv1a("appa yep yep"), PHRASES[0]);
        assert_eq!(fnv1a("appa yip yip"), PHRASES[1]);
        for input in ["Appa yep yep", "APPA, YIP YIP!", "  appa   yep yep.  "] {
            let reply = handle_phrase(input, "Gauri").expect(input);
            assert!(reply.line.ends_with("Gauri carries on."));
            assert!(!reply.line.contains('!'));
        }
        assert!(handle_phrase("appa", "Nandi").is_none());
        assert!(handle_phrase("approve all", "Nandi").is_none());
    }

    #[test]
    fn the_phrase_only_returns_words() {
        // The whole response is one line of text: no action, target or id
        let reply = handle_phrase("appa yip yip", "Nandi").unwrap();
        let json = serde_json::to_value(&reply).unwrap();
        assert_eq!(
            json.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["line"]
        );
    }
}
