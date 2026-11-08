//! Studio guides: short, task-shaped Markdown on how to do things in
//! Nandi (build an agent, set up a trigger, …), compiled into the binary.
//! They describe the app, not the user's definitions, so they never live
//! in the user's repo. Studio reads the index in its prompt and fetches a
//! guide by ID (`nandi_list_guides`, `nandi_get_guide`).
//!
//! Guides hold judgement and steps; facts (schema keys, tool IDs, model
//! names) stay generated in the prompt. The tests below fail when a guide
//! names a tool or skill.md key that doesn't exist, or when the user guide
//! has no section for a guide (`<!-- studio-guide: <id> -->`): the guides
//! are the source, and the user guide marks where it covers each one.

use std::sync::OnceLock;

/// `src-tauri/resources/studio-guides/*.md`, in prompt order
const SOURCES: &[&str] = &[
    include_str!("../../../resources/studio-guides/build-an-agent.md"),
    include_str!("../../../resources/studio-guides/agent-or-workflow.md"),
    include_str!("../../../resources/studio-guides/write-a-workflow.md"),
    include_str!("../../../resources/studio-guides/set-up-a-trigger.md"),
    include_str!("../../../resources/studio-guides/connect-a-service.md"),
    include_str!("../../../resources/studio-guides/my-notes.md"),
    include_str!("../../../resources/studio-guides/how-nandi-works.md"),
    include_str!("../../../resources/studio-guides/approvals-and-policies.md"),
    include_str!("../../../resources/studio-guides/runtimes-and-sign-in.md"),
    include_str!("../../../resources/studio-guides/requests-and-work.md"),
    include_str!("../../../resources/studio-guides/network-and-delegation.md"),
    include_str!("../../../resources/studio-guides/companion-and-appearance.md"),
    include_str!("../../../resources/studio-guides/use-the-browser.md"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub id: String,
    pub title: String,
    /// One line on when to read it
    pub when: String,
    pub body: String,
}

/// Every guide, parsed once
pub fn all() -> &'static [Guide] {
    static GUIDES: OnceLock<Vec<Guide>> = OnceLock::new();
    GUIDES.get_or_init(|| {
        SOURCES
            .iter()
            .map(|source| parse(source).expect("a Studio guide has bad frontmatter"))
            .collect()
    })
}

pub fn get(id: &str) -> Option<&'static Guide> {
    all().iter().find(|guide| guide.id == id.trim())
}

/// `---` frontmatter with `id`, `title` and `when` (one line each), then
/// the body
fn parse(source: &str) -> Result<Guide, String> {
    let source = source.replace("\r\n", "\n");
    let rest = source
        .strip_prefix("---\n")
        .ok_or("no frontmatter".to_string())?;
    let (frontmatter, body) = rest
        .split_once("\n---\n")
        .ok_or("unclosed frontmatter".to_string())?;
    let field = |key: &str| {
        frontmatter
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}:")))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing `{key}`"))
    };
    Ok(Guide {
        id: field("id")?,
        title: field("title")?,
        when: field("when")?,
        body: body.trim().to_string(),
    })
}

/// The prompt's index: one line per guide, from the files
pub fn index_lines() -> String {
    all()
        .iter()
        .map(|guide| format!("- `{}`: {}", guide.id, guide.when))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Names a guide (or a playbook) may put in backticks: built-in, Studio,
/// delegation, plan and playbook tools, skill.md and policy keys, and the
/// built-in connectors
#[cfg(test)]
pub(crate) fn known_names() -> std::collections::BTreeSet<String> {
    use crate::services::domain::agent::SKILL_FRONTMATTER_KEYS;
    let mut known: std::collections::BTreeSet<String> =
        crate::services::tools::builtin::builtin_tools()
            .into_iter()
            .map(|t| t.name)
            .collect();
    known.extend(super::tools::specs().into_iter().map(|s| s.name));
    known.insert(crate::services::a2a::tool::NAME.to_string());
    known.extend(
        crate::services::run_plan::tool::specs()
            .into_iter()
            .map(|s| s.name),
    );
    known.extend(
        crate::services::playbooks::specs()
            .into_iter()
            .map(|s| s.name),
    );
    known.extend(
        SKILL_FRONTMATTER_KEYS
            .iter()
            .map(|(key, _)| key.to_string()),
    );
    known.extend(
        crate::services::domain::policy::POLICY_KEYS
            .iter()
            .map(|key| key.to_string()),
    );
    known.extend(crate::services::connectors::builtin_ids());
    known
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_guide_parses_with_a_unique_id() {
        let ids: BTreeSet<&str> = all().iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids.len(), SOURCES.len());
        for guide in all() {
            assert!(!guide.body.is_empty(), "{} is empty", guide.id);
            assert!(
                !guide.when.contains(':'),
                "{}: `when` is one plain line",
                guide.id
            );
            assert_eq!(get(&guide.id), Some(guide));
        }
        assert_eq!(get("nope"), None);
        assert!(parse("no frontmatter").is_err());
        assert!(
            parse("---\nid: x\ntitle: X\n---\nbody").is_err(),
            "no `when`"
        );
    }

    /// Guides must not invent facts the prompt generates: a backticked
    /// snake_case name is a real tool (built-in or Studio's), delegation,
    /// skill.md or policy key, or built-in connector
    #[test]
    fn guides_name_only_real_tools_and_keys() {
        let known = known_names();
        let snake =
            regex::Regex::new(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)(?:\.[a-z_]+)?:?`").unwrap();
        for guide in all() {
            for name in snake.captures_iter(&guide.body).map(|c| c[1].to_string()) {
                assert!(known.contains(&name), "{} names unknown `{name}`", guide.id);
            }
        }
    }

    #[test]
    fn guides_name_no_models() {
        // Model names change; the prompt lists the usable ones from code
        let model =
            regex::Regex::new(r"(claude|gpt|deepseek|ollama|gemini)[-:][A-Za-z0-9]").unwrap();
        for guide in all() {
            if let Some(found) = model.find(&guide.body) {
                panic!("{} names a model: {}", guide.id, found.as_str());
            }
        }
    }

    #[test]
    fn the_user_guide_covers_every_guide() {
        let userguide = include_str!("../../../../docs/userguide/userguide.md");
        for guide in all() {
            let anchor = format!("<!-- studio-guide: {} -->", guide.id);
            assert!(
                userguide.contains(&anchor),
                "docs/userguide/userguide.md has no {anchor}"
            );
        }
    }
}
