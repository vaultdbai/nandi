//! Agents that ship with Nandi itself, not with a pack.
//!
//! Each is written into the repo as an ordinary local agent
//! (`.nandi/agents/<id>/`) whenever its folder is missing, so the user owns
//! and edits it like one they made; Backup commits its history. It can't
//! be deleted: features such as My notes and `ask-me` rely on it.

use std::path::{Path, PathBuf};

use crate::services::repos::fs_atomic::write_if_changed;
use crate::services::repos::layout::{INSTRUCTIONS_FILE, SKILL_FILE};
use crate::services::repos::RepoLayout;

/// The agent that keeps My notes and answers for the user
pub const CHIEF_OF_STAFF: &str = "chief-of-staff";

/// An agent that ships with Nandi: its ID and the files written into a new
/// folder
pub struct BuiltinAgent {
    pub id: &'static str,
    files: &'static [(&'static str, &'static str)],
}

/// Every built-in agent, in the order the sidebar lists them
pub const BUILTIN_AGENTS: &[BuiltinAgent] = &[BuiltinAgent {
    id: CHIEF_OF_STAFF,
    files: &[
        (
            SKILL_FILE,
            include_str!("../../resources/builtin-agents/chief-of-staff/skill.md"),
        ),
        (
            INSTRUCTIONS_FILE,
            include_str!("../../resources/builtin-agents/chief-of-staff/instructions.md"),
        ),
    ],
}];

pub fn is_builtin(id: &str) -> bool {
    position(id).is_some()
}

/// Where a built-in agent comes in [`BUILTIN_AGENTS`], for listing them first
pub fn position(id: &str) -> Option<usize> {
    BUILTIN_AGENTS.iter().position(|agent| agent.id == id)
}

/// Writes each built-in agent whose `skill.md` the repo lacks, and returns
/// the files written. An existing folder is the user's and stays as it is.
pub fn seed(layout: &RepoLayout) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for agent in BUILTIN_AGENTS {
        let id = agent.id;
        if layout.skill_path(id).is_file() {
            continue;
        }
        let dir = layout.agent_dir(id);
        for (name, content) in agent.files {
            let path = dir.join(name);
            if write_new(&path, content)? {
                written.push(path);
            }
        }
        log::info!("Added the built-in agent '{id}'");
    }
    Ok(written)
}

/// Writes `path` unless it exists (a kept `instructions.md` stays).
fn write_new(path: &Path, content: &str) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }
    write_if_changed(path, content.as_bytes())
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(true)
}

/// The "My role" line that says who the user is
const NAME_LINE: &str = "- **Name and title:**";

/// The Chief of Staff's "My role" with a mailbox owner's name and address
/// in its "Name and title" line, so every run knows who "me" is. The
/// template's placeholder is replaced; a line the user wrote gets the
/// address added. `None`: nothing to change (already there, or the user
/// removed the line).
pub fn with_mailbox_identity(
    instructions: &str,
    name: Option<&str>,
    email: &str,
) -> Option<String> {
    let email = email.trim();
    if email.is_empty() {
        return None;
    }
    let mut changed = false;
    let lines: Vec<String> = instructions
        .lines()
        .map(|line| {
            let Some(rest) = line.trim_start().strip_prefix(NAME_LINE) else {
                return line.to_string();
            };
            let rest = rest.trim();
            if rest
                .to_ascii_lowercase()
                .contains(&email.to_ascii_lowercase())
            {
                return line.to_string();
            }
            changed = true;
            let placeholder = rest.is_empty() || rest.starts_with("(e.g.");
            match (placeholder, name.map(str::trim).filter(|n| !n.is_empty())) {
                (true, Some(name)) => format!("{NAME_LINE} {name}, {email}"),
                (true, None) => format!("{NAME_LINE} {email}"),
                (false, _) => format!("{NAME_LINE} {rest}, {email}"),
            }
        })
        .collect();
    if !changed {
        return None;
    }
    let mut text = lines.join("\n");
    if instructions.ends_with('\n') {
        text.push('\n');
    }
    Some(text)
}

/// Writes the mailbox owner into the Chief of Staff's "My role" (see
/// [`with_mailbox_identity`]), through the agent repository
pub fn note_identity(
    store: &crate::services::repo_store::RepoStore,
    name: Option<&str>,
    email: &str,
) -> Result<bool, String> {
    let current = store.skill(CHIEF_OF_STAFF)?.instructions;
    let Some(updated) = with_mailbox_identity(&current, name, email) else {
        return Ok(false);
    };
    store
        .repos()
        .agents
        .save_instructions(CHIEF_OF_STAFF, &updated)
        .map_err(|e| e.to_string())?;
    store.commit(crate::services::domain::EntityKind::Agent, CHIEF_OF_STAFF)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent::Agent;
    use crate::services::domain::AgentId;

    fn layout() -> (tempfile::TempDir, RepoLayout) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        (tmp, layout)
    }

    fn builtin(id: &str) -> &'static BuiltinAgent {
        &BUILTIN_AGENTS[position(id).unwrap()]
    }

    /// A built-in agent's shipped file
    fn file(agent: &BuiltinAgent, name: &str) -> &'static str {
        agent
            .files
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, content)| *content)
            .unwrap()
    }

    fn shipped(id: &str) -> Agent {
        Agent::from_skill_md(AgentId::parse(id).unwrap(), file(builtin(id), SKILL_FILE)).unwrap()
    }

    #[test]
    fn seeds_every_builtin_agent_once() {
        let (_tmp, layout) = layout();
        let written = seed(&layout).unwrap();
        assert_eq!(written.len(), 2, "{written:?}");
        assert!(layout.skill_path(CHIEF_OF_STAFF).is_file());
        assert!(layout.instructions_path(CHIEF_OF_STAFF).is_file());
        assert!(seed(&layout).unwrap().is_empty(), "nothing to add again");
    }

    #[test]
    fn the_chief_of_staff_is_listed_first() {
        assert_eq!(position(CHIEF_OF_STAFF), Some(0));
        assert!(is_builtin(CHIEF_OF_STAFF));
        assert!(!is_builtin("writer"));
    }

    #[test]
    fn keeps_the_users_edits() {
        let (_tmp, layout) = layout();
        seed(&layout).unwrap();
        let skill = layout.skill_path(CHIEF_OF_STAFF);
        let edited = std::fs::read_to_string(&skill).unwrap() + "\nAlways answer in French.\n";
        std::fs::write(&skill, &edited).unwrap();
        seed(&layout).unwrap();
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
    }

    #[test]
    fn restores_a_removed_skill_but_keeps_the_role() {
        let (_tmp, layout) = layout();
        seed(&layout).unwrap();
        std::fs::write(layout.instructions_path(CHIEF_OF_STAFF), "# My role\nDev\n").unwrap();
        std::fs::remove_file(layout.skill_path(CHIEF_OF_STAFF)).unwrap();
        assert_eq!(seed(&layout).unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(layout.instructions_path(CHIEF_OF_STAFF)).unwrap(),
            "# My role\nDev\n"
        );
    }

    /// The words of a skill.md body (after the frontmatter)
    fn body_words(skill_md: &str) -> usize {
        let (_, body) = crate::services::domain::agent::split_frontmatter(skill_md).unwrap();
        body.split_whitespace().count()
    }

    #[test]
    fn the_shipped_skills_parse_and_stay_short() {
        assert_eq!(shipped(CHIEF_OF_STAFF).name(), "Chief of Staff");
        for agent in BUILTIN_AGENTS {
            let words = body_words(file(agent, SKILL_FILE));
            assert!(words <= 900, "{}: {words} words", agent.id);
            assert!(
                shipped(agent.id).model().is_none(),
                "{} sets no model",
                agent.id
            );
        }
    }

    #[test]
    fn the_chief_of_staff_browses_with_the_visible_browser() {
        let agent = shipped(CHIEF_OF_STAFF);
        let caps = agent.capabilities();
        let pilots: Vec<String> = caps
            .pilots()
            .iter()
            .map(|p| p.as_str().to_string())
            .collect();
        assert_eq!(pilots, ["browser"], "the visible browser, never headless");
        // Clicking and typing run; submitting, secrets, uploads and paying ask
        assert_eq!(caps.pilot_options("browser").unwrap().allow, ["interact"]);
        assert!(caps.network().allow.is_empty(), "the user allows each site");
    }

    #[test]
    fn the_mailbox_owner_goes_into_my_role() {
        let template = file(builtin(CHIEF_OF_STAFF), INSTRUCTIONS_FILE);
        let filled =
            with_mailbox_identity(template, Some("Dev Chechi"), "dev@company.com").unwrap();
        assert!(filled.contains("- **Name and title:** Dev Chechi, dev@company.com\n"));
        assert!(!filled.contains("(e.g. \"Dev, platform engineering lead\")"));
        // The rest of the template stays
        assert!(filled.contains("- **Team and manager:**"));
        // Once there, nothing changes; a second mailbox adds its address
        assert_eq!(
            with_mailbox_identity(&filled, Some("Dev"), "DEV@company.com"),
            None
        );
        let both = with_mailbox_identity(&filled, Some("Dev"), "dev@gmail.com").unwrap();
        assert!(both.contains("Dev Chechi, dev@company.com, dev@gmail.com"));
        // The user's own words are kept
        let own = "# My role\n- **Name and title:** Dev, platform lead\n";
        assert_eq!(
            with_mailbox_identity(own, None, "dev@company.com").unwrap(),
            "# My role\n- **Name and title:** Dev, platform lead, dev@company.com\n"
        );
        // A role without the line is left alone
        assert_eq!(
            with_mailbox_identity("# My role\nDev\n", None, "d@x.com"),
            None
        );
    }
}
