//! Playbooks: an agent's skill files. Each is `skills/<name>.md` next to
//! its `skill.md`: frontmatter with `name` (the file stem), `description`
//! and `when` (one line each), then the steps in Markdown.
//!
//! The run's prompt carries only the index (name, description, when); the
//! agent fetches a body when the work calls for it (`skill_get`), so an
//! agent can know many playbooks and still keep its prompt short.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::agent::split_frontmatter;
use super::{check_len, has_conflict_markers, DomainError, DomainResult, CONFLICT_MARKERS};

/// Most words a playbook's body may have (like a pack agent's skill.md)
pub const MAX_PLAYBOOK_WORDS: usize = 900;
/// Longest name, in characters
pub const MAX_NAME_CHARS: usize = 64;
/// Longest `description` or `when`, in characters: the index stays short
pub const MAX_LINE_CHARS: usize = 200;
/// The frontmatter keys a playbook has, all required
pub const PLAYBOOK_KEYS: &[&str] = &["name", "description", "when"];

/// One playbook, valid by construction
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Playbook {
    name: String,
    description: String,
    when: String,
    body: String,
}

/// The frontmatter as written
#[derive(Serialize)]
struct Frontmatter<'a> {
    name: &'a str,
    description: &'a str,
    when: &'a str,
}

/// A kebab-case name: lowercase letters and digits, single hyphens between
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

impl Playbook {
    /// A playbook from its parts, trimmed and checked
    pub fn new(name: &str, description: &str, when: &str, body: &str) -> DomainResult<Self> {
        let name = name.trim();
        let entity = format!("playbook '{name}'");
        if !is_valid_name(name) {
            return Err(DomainError::invalid(
                entity,
                "name",
                format!(
                    "should be kebab-case (lowercase letters, digits and single hyphens), at most {MAX_NAME_CHARS} characters"
                ),
            ));
        }
        let line = |field: &'static str, value: &str| -> DomainResult<String> {
            let value = value.trim();
            if value.is_empty() {
                return Err(DomainError::invalid(&entity, field, "is required"));
            }
            if value.contains('\n') {
                return Err(DomainError::invalid(&entity, field, "should be one line"));
            }
            check_len(&entity, field, value, MAX_LINE_CHARS)?;
            Ok(value.to_string())
        };
        let description = line("description", description)?;
        let when = line("when", when)?;
        let body = body.trim().replace("\r\n", "\n");
        if body.is_empty() {
            return Err(DomainError::invalid(
                &entity,
                "body",
                "is empty: write the steps",
            ));
        }
        let count = words(&body);
        if count > MAX_PLAYBOOK_WORDS {
            return Err(DomainError::invalid(
                &entity,
                "body",
                format!("has {count} words; keep it to {MAX_PLAYBOOK_WORDS} or fewer"),
            ));
        }
        if has_conflict_markers(&body) {
            return Err(DomainError::invalid(&entity, "body", CONFLICT_MARKERS));
        }
        Ok(Self {
            name: name.to_string(),
            description,
            when,
            body,
        })
    }

    /// Parses `skills/<stem>.md`. The `name` must be the file's stem, and
    /// only the playbook keys are allowed (no `model:`: a playbook runs on
    /// its agent's model).
    pub fn from_md(stem: &str, content: &str) -> DomainResult<Self> {
        let what = || format!("playbook skills/{stem}.md");
        if has_conflict_markers(content) {
            return Err(DomainError::parse(what(), CONFLICT_MARKERS));
        }
        let (yaml, body) = split_frontmatter(content).map_err(|e| DomainError::parse(what(), e))?;
        let yaml = yaml.filter(|y| !y.trim().is_empty()).ok_or_else(|| {
            DomainError::parse(what(), "has no frontmatter (name, description, when)")
        })?;
        let fields: BTreeMap<String, serde_yaml::Value> =
            serde_yaml::from_str(yaml).map_err(|e| DomainError::parse(what(), e))?;
        if let Some(key) = fields
            .keys()
            .find(|key| !PLAYBOOK_KEYS.contains(&key.as_str()))
        {
            let reason = if key == "model" {
                "sets `model:`; a playbook runs on its agent's model".to_string()
            } else {
                format!("has an unknown key `{key}` (allowed: name, description, when)")
            };
            return Err(DomainError::parse(what(), reason));
        }
        let text = |key: &str| match fields.get(key) {
            Some(serde_yaml::Value::String(value)) => Ok(value.as_str()),
            Some(_) => Err(DomainError::parse(
                what(),
                format!("`{key}` should be text"),
            )),
            None => Err(DomainError::parse(what(), format!("has no `{key}`"))),
        };
        let name = text("name")?;
        if name.trim() != stem {
            return Err(DomainError::parse(
                what(),
                format!(
                    "is named '{}'; the name must be the file name ('{stem}')",
                    name.trim()
                ),
            ));
        }
        Self::new(name, text("description")?, text("when")?, body)
    }

    /// The file's contents
    pub fn to_md(&self) -> String {
        let yaml = serde_yaml::to_string(&Frontmatter {
            name: &self.name,
            description: &self.description,
            when: &self.when,
        })
        .unwrap_or_default();
        format!("---\n{yaml}---\n\n{}\n", self.body)
    }

    /// Its line in the prompt's index
    pub fn index_line(&self) -> String {
        format!(
            "- `{}`: {} Use when {}",
            self.name,
            sentence(&self.description),
            sentence(&lower_first(&self.when))
        )
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn when(&self) -> &str {
        &self.when
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn words(&self) -> usize {
        words(&self.body)
    }
}

/// `text` ending with a full stop
fn sentence(text: &str) -> String {
    if text.ends_with(['.', '!', '?']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// "Paying a bill" → "paying a bill", but "I…" and "URL…" stay
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && second.is_lowercase() => {
            first.to_lowercase().chain(text.chars().skip(1)).collect()
        }
        _ => text.to_string(),
    }
}

/// Fails when two playbooks share a name
pub fn check_unique(playbooks: &[Playbook]) -> DomainResult<()> {
    let mut seen = BTreeSet::new();
    for playbook in playbooks {
        if !seen.insert(playbook.name()) {
            return Err(DomainError::Rule(format!(
                "Two playbooks are named '{}': each needs its own name",
                playbook.name()
            )));
        }
    }
    Ok(())
}

/// The prompt section that lists an agent's playbooks; empty without any
pub fn prompt_index(playbooks: &[Playbook]) -> String {
    if playbooks.is_empty() {
        return String::new();
    }
    let lines = playbooks
        .iter()
        .map(Playbook::index_line)
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\n## Your playbooks\nStep-by-step guides for work you do. When a task matches one, read it first (`skill_get` with its name) and follow it.\n{lines}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAY: &str = "---\nname: pay-a-bill\ndescription: 'Pay a bill on a website: find it, fill it, hand over'\nwhen: The user asks to pay a bill\n---\n\n1. Plan first.\n2. Open the site.\n";

    #[test]
    fn parses_and_writes_back() {
        let playbook = Playbook::from_md("pay-a-bill", PAY).unwrap();
        assert_eq!(playbook.name(), "pay-a-bill");
        assert_eq!(
            playbook.description(),
            "Pay a bill on a website: find it, fill it, hand over"
        );
        assert_eq!(playbook.when(), "The user asks to pay a bill");
        assert_eq!(playbook.body(), "1. Plan first.\n2. Open the site.");
        let again = Playbook::from_md("pay-a-bill", &playbook.to_md()).unwrap();
        assert_eq!(again, playbook);
    }

    #[test]
    fn handles_crlf_and_a_bom() {
        let crlf = format!("\u{feff}{}", PAY.replace('\n', "\r\n"));
        let playbook = Playbook::from_md("pay-a-bill", &crlf).unwrap();
        assert_eq!(playbook.body(), "1. Plan first.\n2. Open the site.");
    }

    #[test]
    fn the_name_is_kebab_case_and_the_file_name() {
        for good in ["a", "web-task", "step-2", "x1-y2"] {
            assert!(is_valid_name(good), "{good}");
        }
        for bad in ["", "Web", "web_task", "-a", "a-", "a--b", "a b", "ä"] {
            assert!(!is_valid_name(bad), "{bad}");
        }
        assert!(!is_valid_name(&"a".repeat(MAX_NAME_CHARS + 1)));
        let err = Playbook::from_md("other", PAY).unwrap_err().to_string();
        assert!(err.contains("must be the file name"), "{err}");
        assert!(Playbook::new("Pay Bill", "d", "w", "b").is_err());
    }

    #[test]
    fn every_field_is_required_and_one_line() {
        assert!(Playbook::new("a", "", "w", "b").is_err());
        assert!(Playbook::new("a", "d", " ", "b").is_err());
        assert!(Playbook::new("a", "d", "w", "  ").is_err());
        assert!(Playbook::new("a", "two\nlines", "w", "b").is_err());
        assert!(Playbook::new("a", &"d".repeat(MAX_LINE_CHARS + 1), "w", "b").is_err());
        let missing = "---\nname: a\ndescription: d\n---\nbody";
        assert!(Playbook::from_md("a", missing)
            .unwrap_err()
            .to_string()
            .contains("`when`"));
        assert!(Playbook::from_md("a", "no frontmatter").is_err());
        let not_text = "---\nname: a\ndescription: [x]\nwhen: w\n---\nbody";
        assert!(Playbook::from_md("a", not_text).is_err());
    }

    #[test]
    fn the_body_has_at_most_900_words() {
        let fits = "word ".repeat(MAX_PLAYBOOK_WORDS);
        assert_eq!(
            Playbook::new("a", "d", "w", &fits).unwrap().words(),
            MAX_PLAYBOOK_WORDS
        );
        let long = "word ".repeat(MAX_PLAYBOOK_WORDS + 1);
        let err = Playbook::new("a", "d", "w", &long).unwrap_err().to_string();
        assert!(err.contains("901 words"), "{err}");
    }

    #[test]
    fn no_model_and_no_unknown_keys() {
        let model = "---\nname: a\ndescription: d\nwhen: w\nmodel: claude:x\n---\nbody";
        let err = Playbook::from_md("a", model).unwrap_err().to_string();
        assert!(err.contains("model"), "{err}");
        let other = "---\nname: a\ndescription: d\nwhen: w\ntools: [x]\n---\nbody";
        assert!(Playbook::from_md("a", other)
            .unwrap_err()
            .to_string()
            .contains("unknown key `tools`"));
    }

    #[test]
    fn refuses_conflict_markers() {
        let body = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n";
        assert!(Playbook::new("a", "d", "w", body).is_err());
    }

    #[test]
    fn names_are_unique() {
        let a = Playbook::new("a", "d", "w", "b").unwrap();
        let b = Playbook::new("b", "d", "w", "b").unwrap();
        assert!(check_unique(&[a.clone(), b]).is_ok());
        assert!(check_unique(&[a.clone(), a]).is_err());
    }

    #[test]
    fn the_index_is_one_line_each() {
        assert_eq!(prompt_index(&[]), "");
        let playbook = Playbook::from_md("pay-a-bill", PAY).unwrap();
        let index = prompt_index(&[playbook]);
        assert!(index.contains("## Your playbooks"));
        assert!(index.contains("`skill_get`"));
        assert!(index.contains(
            "- `pay-a-bill`: Pay a bill on a website: find it, fill it, hand over. Use when the user asks to pay a bill."
        ));
        assert!(!index.contains("Plan first"), "bodies stay out");
        let acronym = Playbook::new("a", "Done.", "URLs come in", "b").unwrap();
        assert_eq!(acronym.index_line(), "- `a`: Done. Use when URLs come in.");
    }
}
