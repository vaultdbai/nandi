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

use crate::services::domain::agent::Agent;
use crate::services::domain::playbook::Playbook;
use crate::services::domain::{AgentId, EntityKind};
use crate::services::repo_service::GitService;
use crate::services::repo_store::RepoStore;
use crate::services::repos::fs_atomic::write_if_changed;
use crate::services::repos::layout::{INSTRUCTIONS_FILE, PLAYBOOKS_DIR, SKILL_FILE};
use crate::services::repos::{RepoLayout, Repository};

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
macro_rules! nandi_playbook {
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
        nandi_playbook!("web-task"),
        nandi_playbook!("pay-a-bill"),
        nandi_playbook!("sign-in-and-accounts"),
        nandi_playbook!("delegate-work"),
        nandi_playbook!("build-a-workflow"),
        nandi_playbook!("email-triage"),
        nandi_playbook!("meeting-prep"),
        nandi_playbook!("occasions"),
        nandi_playbook!("know-me"),
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

/// The commit that keeps the person's own files in the gold copy's history
/// before a reset to defaults changes them
pub const BEFORE_RESET_COMMIT: &str = "Keep Nandi's files before a reset to defaults";
/// The commit of a reset to defaults
pub const RESET_COMMIT: &str = "Reset Nandi to defaults";

/// The default agent as it ships: its skill.md (with its default settings),
/// the instructions.md template, and its playbooks
fn shipped_default() -> Result<(Agent, Vec<Playbook>), String> {
    let nandi = BUILTIN_AGENTS
        .iter()
        .find(|agent| is_default(agent.id))
        .ok_or("Nandi doesn't ship with this build")?;
    let shipped = |name: &str| {
        nandi
            .files
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, content)| *content)
    };
    let skill = shipped(SKILL_FILE).ok_or("Nandi ships without a skill.md")?;
    let agent = Agent::from_files(
        AgentId::parse(DEFAULT_AGENT).map_err(|e| e.to_string())?,
        skill,
        shipped(INSTRUCTIONS_FILE),
    )
    .map_err(|e| format!("Nandi's shipped skill.md doesn't load: {e}"))?;
    let playbooks = nandi
        .playbooks
        .iter()
        .map(|(name, content)| Playbook::from_md(name, content))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("A shipped playbook doesn't load: {e}"))?;
    Ok((agent, playbooks))
}

/// Puts Nandi back as it ships (owner decision N-5): its skill.md, which
/// holds its model, network and other settings; the instructions.md
/// template; and its playbooks, so playbooks the person added go. Its
/// memory, runs, grants and triggers stay.
///
/// Like any gold-copy change, the files are written through the agent
/// repository and committed. Two git commits: first the person's files as
/// they are ([`BEFORE_RESET_COMMIT`]), so nothing they wrote is lost, then
/// the reset ([`RESET_COMMIT`]). Without a git repository nothing changes.
pub fn reset_default(store: &RepoStore, git: &GitService) -> Result<(), String> {
    let agents = &store.repos().agents;
    let owner = store.repos().load_manifest().ok().and_then(|m| m.owner);
    let relative = |paths: &[PathBuf]| -> Vec<String> {
        paths
            .iter()
            .filter_map(|path| store.layout().relative(path))
            .collect()
    };

    let mut files = agents.definition_files(DEFAULT_AGENT)?;
    git.commit_files(&relative(&files), BEFORE_RESET_COMMIT, owner.as_deref())?;

    let (agent, playbooks) = shipped_default()?;
    agents.save(&agent)?;
    agents.replace_playbooks(DEFAULT_AGENT, &playbooks)?;
    store.commit(EntityKind::Agent, DEFAULT_AGENT)?;

    files.extend(agents.definition_files(DEFAULT_AGENT)?);
    files.sort();
    files.dedup();
    git.commit_files(&relative(&files), RESET_COMMIT, owner.as_deref())?;
    log::info!("Reset the default agent '{DEFAULT_AGENT}' to its shipped files");
    Ok(())
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
pub fn note_identity(store: &RepoStore, name: Option<&str>, email: &str) -> Result<bool, String> {
    let current = store.skill(DEFAULT_AGENT)?.instructions;
    let Some(updated) = with_mailbox_identity(&current, name, email) else {
        return Ok(false);
    };
    store
        .repos()
        .agents
        .save_instructions(DEFAULT_AGENT, &updated)
        .map_err(|e| e.to_string())?;
    store.commit(EntityKind::Agent, DEFAULT_AGENT)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A seeded gold copy in a git repository, with its store
    fn gold_copy() -> (tempfile::TempDir, RepoStore, GitService) {
        let (tmp, layout) = layout();
        seed(&layout).unwrap();
        git2::Repository::init(&layout.root).unwrap();
        let git = GitService::new(&layout.root);
        let store = RepoStore::sqlite(
            crate::services::repos::Repos::new(layout),
            crate::db::Db::in_memory(),
        );
        store.load().unwrap();
        (tmp, store, git)
    }

    fn nandi_id() -> AgentId {
        AgentId::parse(DEFAULT_AGENT).unwrap()
    }

    /// The text of `path` in `commit`, or `None` when it isn't there
    fn file_in(repo: &git2::Repository, commit: &git2::Commit, path: &str) -> Option<String> {
        let entry = commit.tree().unwrap().get_path(Path::new(path)).ok()?;
        let blob = repo.find_blob(entry.id()).unwrap();
        Some(String::from_utf8(blob.content().to_vec()).unwrap())
    }

    #[test]
    fn a_reset_restores_the_shipped_files() {
        let (_tmp, store, git) = gold_copy();
        let agents = &store.repos().agents;
        let skill = store.layout().skill_path(DEFAULT_AGENT);
        let shipped_skill = std::fs::read_to_string(&skill).unwrap();
        let renamed = shipped_skill.replacen("name: Nandi", "name: My Nandi", 1);
        assert_ne!(renamed, shipped_skill);
        std::fs::write(&skill, renamed).unwrap();
        store.commit(EntityKind::Agent, DEFAULT_AGENT).unwrap();
        assert_eq!(store.display_name(DEFAULT_AGENT), "My Nandi");
        agents
            .save_instructions(DEFAULT_AGENT, "# My role\nDev\n")
            .unwrap();
        let own = Playbook::new("my-own", "Mine", "Always", "Do it my way.").unwrap();
        agents.save_playbook(DEFAULT_AGENT, &own, None).unwrap();
        agents.delete_playbook(DEFAULT_AGENT, "occasions").unwrap();
        let memory = agents.write_memory(DEFAULT_AGENT, "Likes tea\n").unwrap();

        reset_default(&store, &git).unwrap();

        let (shipped_agent, shipped_playbooks) = shipped_default().unwrap();
        let agent = agents.load(&nandi_id()).unwrap();
        assert_eq!(agent.to_skill_md(), shipped_agent.to_skill_md());
        assert_eq!(
            agent.to_instructions_md(),
            shipped_agent.to_instructions_md()
        );
        assert_eq!(store.display_name(DEFAULT_AGENT), "Nandi", "projected");
        let playbooks = agents.load_playbooks(DEFAULT_AGENT).unwrap().entities;
        let mut expected = shipped_playbooks;
        expected.sort_by(|a, b| a.name().cmp(b.name()));
        assert_eq!(playbooks, expected, "shipped ones back, the added one gone");
        assert_eq!(std::fs::read_to_string(memory).unwrap(), "Likes tea\n");
    }

    #[test]
    fn a_reset_keeps_the_old_files_in_the_history() {
        let (_tmp, store, git) = gold_copy();
        let agents = &store.repos().agents;
        let own = Playbook::new("my-own", "Mine", "Always", "Do it my way.").unwrap();
        agents.save_playbook(DEFAULT_AGENT, &own, None).unwrap();
        agents
            .save_instructions(DEFAULT_AGENT, "# My role\nDev\n")
            .unwrap();

        reset_default(&store, &git).unwrap();

        let repo = git2::Repository::open(&store.layout().root).unwrap();
        let reset = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(reset.message(), Some(RESET_COMMIT));
        let before = reset.parent(0).unwrap();
        assert_eq!(before.message(), Some(BEFORE_RESET_COMMIT));

        let playbook = format!("{}/skills/my-own.md", agent_path(&store));
        let role = format!("{}/{INSTRUCTIONS_FILE}", agent_path(&store));
        assert!(
            file_in(&repo, &before, &playbook).is_some(),
            "kept in history"
        );
        assert_eq!(
            file_in(&repo, &before, &role).as_deref(),
            Some("# My role\nDev\n")
        );
        assert_eq!(file_in(&repo, &reset, &playbook), None);
        assert!(git.status().unwrap().files.is_empty(), "all committed");
    }

    #[test]
    fn a_reset_needs_the_gold_copys_history() {
        let (_tmp, layout) = layout();
        seed(&layout).unwrap();
        let git = GitService::new(&layout.root);
        let store = RepoStore::sqlite(
            crate::services::repos::Repos::new(layout),
            crate::db::Db::in_memory(),
        );
        store.load().unwrap();
        let skill = store.layout().skill_path(DEFAULT_AGENT);
        let edited = std::fs::read_to_string(&skill).unwrap() + "\nMy own words.\n";
        std::fs::write(&skill, &edited).unwrap();

        let error = reset_default(&store, &git).unwrap_err();
        assert!(error.contains("isn't a git repository"), "{error}");
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
    }

    /// Nandi's folder, relative to the repo root
    fn agent_path(store: &RepoStore) -> String {
        store
            .layout()
            .relative(&store.layout().agent_dir(DEFAULT_AGENT))
            .unwrap()
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
