//! Agents that ship with Nandi itself, not with a pack. Today that is one:
//! the default agent, named Nandi, that routes the ask box, keeps My notes,
//! checks mail and answers `ask-me` and `assign-me` for the user.
//!
//! Each is written into the repo as an ordinary local agent
//! (`.nandi/agents/<id>/`) whenever its folder is missing, so the user owns
//! and edits it like one they made; Backup commits its history. It can't
//! be deleted: features such as My notes and `ask-me` rely on it.
//!
//! [`DEFAULT_AGENT`] is the only place that spells its ID. Other people
//! reach it by its capabilities, never by its ID.
//!
//! Its playbooks (`skills/`) are written with the folder: once, when the
//! agent has no `skills/` folder. Playbooks the user edits or removes stay
//! as they left them.

use std::path::{Path, PathBuf};

use crate::services::repos::fs_atomic::write_if_changed;
use crate::services::repos::layout::{INSTRUCTIONS_FILE, PLAYBOOKS_DIR, SKILL_FILE};
use crate::services::repos::RepoLayout;

/// The ID of the default agent, Nandi: it keeps My notes and answers for
/// the user
pub const DEFAULT_AGENT: &str = "nandi";

/// An agent that ships with Nandi: its ID, the files written into a new
/// folder, and its playbooks by name
pub struct BuiltinAgent {
    pub id: &'static str,
    files: &'static [(&'static str, &'static str)],
    playbooks: &'static [(&'static str, &'static str)],
}

/// A Nandi playbook: `(name, contents)`
macro_rules! chief_playbook {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!(
                "../../resources/builtin-agents/nandi/skills/",
                $name,
                ".md"
            )),
        )
    };
}

/// Every built-in agent
pub const BUILTIN_AGENTS: &[BuiltinAgent] = &[BuiltinAgent {
    id: DEFAULT_AGENT,
    files: &[
        (
            SKILL_FILE,
            include_str!("../../resources/builtin-agents/nandi/skill.md"),
        ),
        (
            INSTRUCTIONS_FILE,
            include_str!("../../resources/builtin-agents/nandi/instructions.md"),
        ),
    ],
    playbooks: &[
        chief_playbook!("web-task"),
        chief_playbook!("pay-a-bill"),
        chief_playbook!("sign-in-and-accounts"),
        chief_playbook!("delegate-work"),
        chief_playbook!("build-a-workflow"),
        chief_playbook!("email-triage"),
        chief_playbook!("meeting-prep"),
        chief_playbook!("occasions"),
        chief_playbook!("know-me"),
    ],
}];

/// Whether `id` is the default agent's ID
pub fn is_default(id: &str) -> bool {
    id == DEFAULT_AGENT
}

/// Writes each built-in agent whose `skill.md` the repo lacks, and its
/// playbooks when it has no `skills/` folder, and returns the files
/// written. Existing files are the user's and stay as they are.
pub fn seed(layout: &RepoLayout) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for agent in BUILTIN_AGENTS {
        let id = agent.id;
        let dir = layout.agent_dir(id);
        if !layout.skill_path(id).is_file() {
            for (name, content) in agent.files {
                let path = dir.join(name);
                if write_new(&path, content)? {
                    written.push(path);
                }
            }
            log::info!("Added the built-in agent '{id}'");
        }
        let skills = dir.join(PLAYBOOKS_DIR);
        if agent.playbooks.is_empty() || skills.exists() {
            continue;
        }
        for (name, content) in agent.playbooks {
            let path = skills.join(format!("{name}.md"));
            if write_new(&path, content)? {
                written.push(path);
            }
        }
        log::info!("Added the playbooks of the built-in agent '{id}'");
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

/// Nandi's "My role" with a mailbox owner's name and address
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

/// Writes the mailbox owner into Nandi's "My role" (see
/// [`with_mailbox_identity`]), through the agent repository
pub fn note_identity(
    store: &crate::services::repo_store::RepoStore,
    name: Option<&str>,
    email: &str,
) -> Result<bool, String> {
    let current = store.skill(DEFAULT_AGENT)?.instructions;
    let Some(updated) = with_mailbox_identity(&current, name, email) else {
        return Ok(false);
    };
    store
        .repos()
        .agents
        .save_instructions(DEFAULT_AGENT, &updated)
        .map_err(|e| e.to_string())?;
    store.commit(crate::services::domain::EntityKind::Agent, DEFAULT_AGENT)?;
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
        BUILTIN_AGENTS.iter().find(|agent| agent.id == id).unwrap()
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
        let playbooks = builtin(DEFAULT_AGENT).playbooks.len();
        assert_eq!(written.len(), 2 + playbooks, "{written:?}");
        assert!(layout.skill_path(DEFAULT_AGENT).is_file());
        assert!(layout.instructions_path(DEFAULT_AGENT).is_file());
        assert!(seed(&layout).unwrap().is_empty(), "nothing to add again");
    }

    #[test]
    fn nandi_is_the_default_agent() {
        assert_eq!(DEFAULT_AGENT, "nandi");
        assert!(is_default(DEFAULT_AGENT));
        assert!(!is_default("writer"));
        assert_eq!(builtin(DEFAULT_AGENT).id, DEFAULT_AGENT);
    }

    /// How a file outside this one would spell the default agent's ID: as
    /// a string, an Inbox thread, a grant, or an agent folder
    const ID_SPELLINGS: &[&str] = &[
        "\"nandi\"",
        "agent:nandi",
        "agent: nandi",
        "agents/nandi",
        "/nandi\"",
    ];

    /// Files where `"nandi"` names the app or the brand, never the agent:
    /// the log file, branch prefixes, the colour palette, the harness
    /// provider, the egress signer, a network label and a browser login
    /// fixture
    const BRAND_FILES: &[&str] = &[
        "lib.rs",
        "types.rs",
        "services/auth/mod.rs",
        "services/domain/settings.rs",
        "services/egress/sigv4.rs",
        "services/harness/acp.rs",
        "services/harness/codex.rs",
        "services/network/mod.rs",
        "services/pilots/browser.rs",
        "services/remote/git_ops.rs",
        "services/remote/mod.rs",
    ];

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    /// Code uses [`DEFAULT_AGENT`], so renaming the agent is a one-line
    /// change. A brand use of `"nandi"` goes in [`BRAND_FILES`].
    #[test]
    fn no_file_outside_this_one_spells_the_default_agent_id() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let mut spelled = Vec::new();
        for path in files {
            let relative = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "services/builtin_agents.rs" {
                continue;
            }
            let brand = BRAND_FILES.contains(&relative.as_str());
            let text = std::fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                let hit = ID_SPELLINGS
                    .iter()
                    .filter(|spelling| !(brand && **spelling == "\"nandi\""))
                    .any(|spelling| line.contains(spelling));
                if hit {
                    spelled.push(format!("{relative}:{}: {}", number + 1, line.trim()));
                }
            }
        }
        assert!(
            spelled.is_empty(),
            "use builtin_agents::DEFAULT_AGENT instead:
{}",
            spelled.join(
                "
"
            )
        );
    }

    #[test]
    fn keeps_the_users_edits() {
        let (_tmp, layout) = layout();
        seed(&layout).unwrap();
        let skill = layout.skill_path(DEFAULT_AGENT);
        let edited = std::fs::read_to_string(&skill).unwrap() + "\nAlways answer in French.\n";
        std::fs::write(&skill, &edited).unwrap();
        seed(&layout).unwrap();
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
    }

    #[test]
    fn restores_a_removed_skill_but_keeps_the_role() {
        let (_tmp, layout) = layout();
        seed(&layout).unwrap();
        std::fs::write(layout.instructions_path(DEFAULT_AGENT), "# My role\nDev\n").unwrap();
        std::fs::remove_file(layout.skill_path(DEFAULT_AGENT)).unwrap();
        assert_eq!(seed(&layout).unwrap().len(), 1, "the playbooks are there");
        assert_eq!(
            std::fs::read_to_string(layout.instructions_path(DEFAULT_AGENT)).unwrap(),
            "# My role\nDev\n"
        );
    }

    #[test]
    fn seeds_the_playbooks_once_and_keeps_the_users_changes() {
        let (_tmp, layout) = layout();
        seed(&layout).unwrap();
        let skills = layout.agent_dir(DEFAULT_AGENT).join(PLAYBOOKS_DIR);
        let pay = skills.join("pay-a-bill.md");
        std::fs::write(&pay, "my own").unwrap();
        std::fs::remove_file(skills.join("occasions.md")).unwrap();
        assert!(seed(&layout).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&pay).unwrap(), "my own");
        assert!(
            !skills.join("occasions.md").exists(),
            "a removed one stays removed"
        );
        // An agent from before playbooks gets them once
        std::fs::remove_dir_all(&skills).unwrap();
        let written = seed(&layout).unwrap();
        assert_eq!(written.len(), builtin(DEFAULT_AGENT).playbooks.len());
        assert!(layout.skill_path(DEFAULT_AGENT).is_file());
    }

    fn shipped_playbooks() -> Vec<crate::services::domain::playbook::Playbook> {
        BUILTIN_AGENTS
            .iter()
            .flat_map(|agent| agent.playbooks.iter())
            .map(|(name, content)| {
                crate::services::domain::playbook::Playbook::from_md(name, content)
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
            })
            .collect()
    }

    #[test]
    fn the_shipped_playbooks_parse_and_stay_short() {
        let playbooks = shipped_playbooks();
        assert_eq!(playbooks.len(), 9);
        crate::services::domain::playbook::check_unique(&playbooks).unwrap();
        for playbook in &playbooks {
            // The domain caps them at 900; these aim for a short read
            assert!(
                playbook.words() <= 600,
                "{}: {} words",
                playbook.name(),
                playbook.words()
            );
        }
    }

    /// A playbook only names tools that exist: a backticked snake_case name
    /// is a known tool (a connector's or the browser's with its prefix) or
    /// a skill.md key, like the Studio guides
    #[test]
    fn the_shipped_playbooks_name_only_real_tools() {
        use crate::services::{a2a, email, requests, workspace};
        let known = crate::services::studio::guides::known_names();
        let browser_tools = [
            "browser_navigate",
            "browser_navigate_back",
            "browser_snapshot",
            "browser_click",
            "browser_type",
            "browser_fill_form",
            "browser_select_option",
            "browser_wait_for",
            "browser_tabs",
            "browser_take_screenshot",
            "browser_close",
            crate::services::pilots::handover::TOOL,
        ];
        // The arguments of the work and people tools (`message_id`, …)
        let arguments: Vec<String> = requests::tool::offered(DEFAULT_AGENT)
            .into_iter()
            .chain(
                [a2a::people::FIND_PEOPLE, a2a::people::MY_REQUESTS]
                    .iter()
                    .filter_map(|n| a2a::people::spec(n)),
            )
            .filter_map(|spec| spec.input_schema["properties"].as_object().cloned())
            .flat_map(|properties| properties.keys().cloned().collect::<Vec<_>>())
            .collect();
        let is_known = |name: &str| {
            known.contains(name)
                || arguments.iter().any(|a| a == name)
                || email::tool::is_mail_tool(name)
                || workspace::tool::is_workspace_tool(name)
                || requests::tool::is_work_tool(name)
                || a2a::people::is_people_tool(name)
                || name == a2a::clarification::ASK_REQUESTER_TOOL
                || crate::services::inbox::tool::is_decision_tool(name)
        };
        let names =
            regex::Regex::new(r"`([a-z][a-z0-9]*(?:__?[a-z0-9]+)+)(?:\.[a-z_]+)?:?`").unwrap();
        for playbook in shipped_playbooks() {
            for name in names
                .captures_iter(playbook.body())
                .map(|c| c[1].to_string())
            {
                let ok = match name.split_once("__") {
                    Some(("pilot_browser", tool)) => browser_tools.contains(&tool),
                    Some(("filesystem", tool)) => known.contains(tool),
                    Some(_) => false,
                    None => is_known(&name),
                };
                assert!(ok, "{} names unknown `{name}`", playbook.name());
            }
        }
    }

    /// The words of a skill.md body (after the frontmatter)
    fn body_words(skill_md: &str) -> usize {
        let (_, body) = crate::services::domain::agent::split_frontmatter(skill_md).unwrap();
        body.split_whitespace().count()
    }

    #[test]
    fn the_shipped_skills_parse_and_stay_short() {
        assert_eq!(shipped(DEFAULT_AGENT).name(), "Nandi");
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
    fn nandi_browses_with_the_visible_browser() {
        let agent = shipped(DEFAULT_AGENT);
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
        let template = file(builtin(DEFAULT_AGENT), INSTRUCTIONS_FILE);
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
