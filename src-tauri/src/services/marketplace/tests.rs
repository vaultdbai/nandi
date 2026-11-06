//! Integration tests: installs from local git fixtures into a real gold
//! copy (temp folder, git repo, in-memory SQLite projection).

use std::fs;
use std::path::{Path, PathBuf};

use super::install::Installer;
use super::manifest::{parse_agent_entry, parse_pack_entry, AgentManifest, PackManifest};
use super::source::tests::fixture_repo;
use crate::db::Db;
use crate::services::domain::installed::InstallKind;
use crate::services::domain::workflow::WorkflowScope;
use crate::services::domain::{AgentId, ChangedBy, WorkflowId};
use crate::services::repo_store::RepoStore;
use crate::services::repos::{RepoError, RepoLayout, Repos, Repository};

struct World {
    tmp: tempfile::TempDir,
    store: RepoStore,
    db: Db,
    installer: Installer,
}

impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        let db = Db::in_memory();
        let store = RepoStore::sqlite(
            Repos::new(RepoLayout::new(&root, tmp.path().join("trash"))),
            db.clone(),
        );
        store.load().unwrap();
        let installer = Installer::new(store.clone(), tmp.path().join("scratch"));
        Self {
            tmp,
            store,
            db,
            installer,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.tmp.path().join(name)
    }

    fn layout(&self) -> &RepoLayout {
        self.store.layout()
    }

    fn projected(&self, table: &str) -> Vec<String> {
        let conn = self.db.conn().unwrap();
        let mut stmt = conn
            .prepare(&format!("SELECT id FROM {table} ORDER BY id"))
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

fn no_progress() -> impl FnMut(super::source::FetchProgress) {
    |_| {}
}

const REVIEW: &str = "id: review\nname: Review\nsteps:\n  - {id: s1, agent: summarizer, input: hi}\n  - {id: s2, agent: citer, input: '{{s1.output}}'}\n";

/// A pack repo: two agents (one with its own workflow), a pack workflow,
/// and a file that must not be installed.
fn pack_files(version: &str, summarizer_body: &str) -> Vec<(String, String)> {
    vec![
        (
            ".nandi/manifest.yaml".into(),
            format!("id: research-pack\nname: Research Pack\nversion: {version}\nagents: [summarizer, citer]\nworkflows: [review]\n"),
        ),
        ("README.md".into(), "# Research".into()),
        (
            "agents/summarizer/skill.md".into(),
            format!("---\nname: Summarizer\nallowed-tools: [read_file]\n---\n{summarizer_body}"),
        ),
        ("agents/citer/skill.md".into(), "---\nname: Citer\n---\nCite.".into()),
        (
            "agents/citer/workflows/citer-daily.yaml".into(),
            "id: citer-daily\nname: Daily\nsteps:\n  - {id: a, agent: citer, input: go}\n".into(),
        ),
        ("workflows/review.yaml".into(), REVIEW.into()),
        ("scripts/postinstall.sh".into(), "rm -rf /".into()),
    ]
}

fn commit(origin: &Path, files: &[(String, String)], tag: &str) -> String {
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    fixture_repo(origin, &refs, tag)
}

fn pack_manifest(origin: &Path, version: &str, min_nandi: &str) -> PackManifest {
    let entry = serde_json::json!({
        "id": "research-pack", "name": "Research Pack", "publisher": "vaultdbai",
        "repo": origin.display().to_string(), "latest_version": version,
        "min_nandi_version": min_nandi,
    });
    parse_pack_entry(&entry.to_string(), "test-registry").unwrap()
}

fn agent_manifest(origin: &Path, id: &str, version: &str, registry: &str) -> AgentManifest {
    let entry = serde_json::json!({
        "id": id, "name": "ArXiv", "publisher": "alice",
        "repo": origin.display().to_string(), "path": format!("agents/{id}"),
        "latest_version": version,
    });
    parse_agent_entry(&entry.to_string(), registry).unwrap()
}

/// An agent repo (alice/nandi-agents style) with `arxiv` in `agents/arxiv`.
fn agent_files(body: &str) -> Vec<(String, String)> {
    vec![
        (
            "agents/arxiv/skill.md".into(),
            format!("---\nname: ArXiv\nsource: github.com/alice/nandi-agents\npublisher: alice\nmodel: claude:claude-sonnet-5\n---\n{body}"),
        ),
        (
            "agents/arxiv/workflows/arxiv-daily.yaml".into(),
            "id: arxiv-daily\nname: Daily papers\nsteps:\n  - {id: a, agent: arxiv, input: today}\n".into(),
        ),
        ("agents/other/skill.md".into(), "---\nname: Other\n---\n".into()),
    ]
}

#[test]
fn installs_a_pack_from_a_local_fixture_into_the_projection() {
    let world = World::new();
    let origin = world.path("origin");
    commit(&origin, &pack_files("1.0.0", "Summarize."), "v1.0.0");

    let info = world
        .installer
        .install_pack(
            &pack_manifest(&origin, "1.0.0", "0.1.0"),
            None,
            &mut no_progress(),
        )
        .unwrap();
    assert_eq!(info.agents, ["summarizer", "citer"]);
    assert_eq!(world.projected("agents"), ["citer", "summarizer"]);
    assert_eq!(world.projected("workflows"), ["citer-daily", "review"]);

    let pack_dir = world.layout().pack_dir("research-pack");
    assert!(pack_dir.join("agents/summarizer/skill.md").is_file());
    assert!(pack_dir.join(".nandi/manifest.yaml").is_file());
    assert!(!pack_dir.join("scripts").exists(), "only allowlisted files");
    let installed = world.store.repos().vendor.load_installed().unwrap();
    let entry = installed.pack("research-pack").unwrap();
    assert_eq!(entry.git_ref, "v1.0.0");
    assert_eq!(entry.commit, info.commit);
    assert!(!entry.signature_verified);

    // Installed agents and workflows are read-only
    let repos = world.store.repos();
    let agent = repos
        .agents
        .load(&AgentId::parse("summarizer").unwrap())
        .unwrap();
    assert!(matches!(
        repos.agents.save(&agent),
        Err(RepoError::ReadOnly { .. })
    ));
    let review = repos
        .workflows
        .load(&WorkflowId::parse("review").unwrap())
        .unwrap();
    assert!(matches!(
        repos.workflows.save(&review),
        Err(RepoError::ReadOnly { .. })
    ));

    // Installing again, or something with the same IDs, is refused
    let again = world.installer.install_pack(
        &pack_manifest(&origin, "1.0.0", "0.1.0"),
        None,
        &mut no_progress(),
    );
    assert!(again.unwrap_err().contains("already installed"));

    // A watcher-style reload after the install changes nothing
    assert!(!world.store.rescan_all(ChangedBy::Watcher));
}

#[test]
fn refuses_entries_and_packs_that_need_a_newer_nandi() {
    let world = World::new();
    let origin = world.path("origin");
    commit(&origin, &pack_files("1.0.0", "x"), "v1.0.0");
    let err = world
        .installer
        .install_pack(
            &pack_manifest(&origin, "1.0.0", "99.0.0"),
            None,
            &mut no_progress(),
        )
        .unwrap_err();
    assert!(err.contains("needs Nandi 99.0.0"), "{err}");

    // The pack's own manifest says so too
    let mut files = pack_files("1.0.0", "x");
    files[0].1.push_str("min_nandi_version: 99.0.0\n");
    commit(&origin, &files, "v1.0.1");
    let mut manifest = pack_manifest(&origin, "1.0.1", "0.1.0");
    manifest.git_ref = "v1.0.1".into();
    let err = world
        .installer
        .install_pack(&manifest, None, &mut no_progress())
        .unwrap_err();
    assert!(err.contains("needs Nandi"), "{err}");
    assert!(
        !world.layout().marketplace_dir().exists(),
        "nothing written"
    );
}

#[test]
fn refuses_ids_that_are_already_taken() {
    let world = World::new();
    let mine = world.layout().skill_path("summarizer");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "---\nname: Mine\n---\n").unwrap();
    world.store.rescan_all(ChangedBy::Watcher);

    let origin = world.path("origin");
    commit(&origin, &pack_files("1.0.0", "x"), "v1.0.0");
    let err = world
        .installer
        .install_pack(
            &pack_manifest(&origin, "1.0.0", "0.1.0"),
            None,
            &mut no_progress(),
        )
        .unwrap_err();
    assert!(err.contains("agent 'summarizer'"), "{err}");
}

#[test]
fn fork_copies_the_folder_rewrites_frontmatter_and_drops_the_entry() {
    let world = World::new();
    let origin = world.path("alice");
    commit(&origin, &agent_files("Search arXiv."), "v1.0.0");
    world
        .installer
        .install_agent(
            &agent_manifest(&origin, "arxiv", "1.0.0", "r"),
            None,
            &mut no_progress(),
        )
        .unwrap();
    let vendored = world.layout().vendored_agent_dir("alice", "arxiv");
    assert!(vendored.join("workflows/arxiv-daily.yaml").is_file());

    let forked = world
        .installer
        .fork_agent("arxiv", "my-arxiv", "My ArXiv")
        .unwrap();
    assert!(forked.warnings.is_empty());
    assert_eq!(forked.agent.name(), "My ArXiv");
    assert!(!vendored.exists(), "the vendored copy moved");
    let skill = fs::read_to_string(world.layout().skill_path("my-arxiv")).unwrap();
    assert!(skill.contains("name: My ArXiv"));
    assert!(skill.contains("model: claude:claude-sonnet-5"));
    assert!(
        !skill.contains("source:") && !skill.contains("publisher:"),
        "{skill}"
    );
    let installed = world.store.repos().vendor.load_installed().unwrap();
    assert!(installed.agent("arxiv").is_none());

    // Its own workflow follows it
    let repos = world.store.repos();
    let daily = repos
        .workflows
        .load(&WorkflowId::parse("arxiv-daily").unwrap())
        .unwrap();
    assert_eq!(daily.steps()[0].performer(), "my-arxiv");
    assert_eq!(
        daily.scope(),
        &WorkflowScope::Agent(AgentId::parse("my-arxiv").unwrap())
    );
    assert_eq!(world.projected("agents"), ["my-arxiv"]);
    assert_eq!(world.projected("workflows"), ["arxiv-daily"]);

    // Now it's editable, and the marketplace won't touch it
    let mut agent = repos
        .agents
        .load(&AgentId::parse("my-arxiv").unwrap())
        .unwrap();
    agent.update_profile("Renamed", "").unwrap();
    repos.agents.save(&agent).unwrap();
    let err = world
        .installer
        .uninstall(InstallKind::Agent, "my-arxiv")
        .unwrap_err();
    assert!(err.contains("your own agent"), "{err}");
    let (updates, _) = world.installer.check_for_updates(&|_| None).unwrap();
    assert!(updates.is_empty(), "updates stop for a fork");
    assert!(world
        .installer
        .fork_agent("my-arxiv", "again", "Again")
        .is_err());
}

#[test]
fn forking_a_pack_agent_keeps_the_pack_consistent() {
    let world = World::new();
    let origin = world.path("origin");
    commit(&origin, &pack_files("1.0.0", "Summarize."), "v1.0.0");
    world
        .installer
        .install_pack(
            &pack_manifest(&origin, "1.0.0", "0.1.0"),
            None,
            &mut no_progress(),
        )
        .unwrap();

    let forked = world
        .installer
        .fork_agent("summarizer", "my-summarizer", "Mine")
        .unwrap();
    assert_eq!(
        forked.warnings,
        ["workflow 'review' still uses 'summarizer'"]
    );
    let installed = world.store.repos().vendor.load_installed().unwrap();
    let pack = installed.pack("research-pack").unwrap();
    assert_eq!(pack.forked_out[0].as_str(), "summarizer");
    let items = world.installer.list_installed().unwrap();
    assert!(!items[0].modified, "a fork isn't a modification");

    // An update leaves the forked agent out
    commit(&origin, &pack_files("1.1.0", "Summarize better."), "v1.1.0");
    world
        .installer
        .apply_update("research-pack", "v1.1.0", None, None, &mut no_progress())
        .unwrap();
    assert!(!world
        .layout()
        .pack_dir("research-pack")
        .join("agents/summarizer")
        .exists());
    assert_eq!(world.projected("agents"), ["citer", "my-summarizer"]);
}

#[test]
fn updates_are_reported_and_refused_over_local_changes() {
    let world = World::new();
    let origin = world.path("alice");
    commit(&origin, &agent_files("v1"), "v1.0.0");
    world
        .installer
        .install_agent(
            &agent_manifest(&origin, "arxiv", "1.0.0", "r"),
            None,
            &mut no_progress(),
        )
        .unwrap();
    let (updates, failures) = world.installer.check_for_updates(&|_| None).unwrap();
    assert!(updates.is_empty() && failures.is_empty());

    commit(&origin, &agent_files("v2"), "v1.1.0");
    let (updates, _) = world.installer.check_for_updates(&|_| None).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].latest_version, "1.1.0");
    assert_eq!(updates[0].latest_ref, "v1.1.0");

    // A local edit blocks the update, naming the file
    let skill = world
        .layout()
        .vendored_agent_dir("alice", "arxiv")
        .join("skill.md");
    let original = fs::read_to_string(&skill).unwrap();
    fs::write(&skill, "---\nname: Hacked\n---\n").unwrap();
    assert!(world.installer.list_installed().unwrap()[0].modified);
    let err = world
        .installer
        .apply_update("arxiv", "v1.1.0", None, None, &mut no_progress())
        .unwrap_err();
    assert!(err.contains("skill.md"), "{err}");

    // Undone: the update applies
    fs::write(&skill, original).unwrap();
    let applied = world
        .installer
        .apply_update("arxiv", "v1.1.0", None, None, &mut no_progress())
        .unwrap();
    assert_eq!(
        (applied.from_version.as_str(), applied.to_version.as_str()),
        ("1.0.0", "1.1.0")
    );
    assert!(fs::read_to_string(&skill).unwrap().ends_with("v2"));
    let installed = world.store.repos().vendor.load_installed().unwrap();
    assert_eq!(installed.agent("arxiv").unwrap().version, "1.1.0");
    let (updates, _) = world.installer.check_for_updates(&|_| None).unwrap();
    assert!(updates.is_empty());
}

#[test]
fn uninstall_removes_files_entry_and_projection_but_not_what_is_in_use() {
    let world = World::new();
    let origin = world.path("origin");
    commit(&origin, &pack_files("1.0.0", "x"), "v1.0.0");
    world
        .installer
        .install_pack(
            &pack_manifest(&origin, "1.0.0", "0.1.0"),
            None,
            &mut no_progress(),
        )
        .unwrap();

    let err = world
        .installer
        .uninstall(InstallKind::Agent, "citer")
        .unwrap_err();
    assert!(err.contains("came with the pack"), "{err}");

    // The user's own workflow uses a pack agent: refused
    let mine = world.layout().workflow_path("mine", &WorkflowScope::Global);
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(
        &mine,
        "id: mine\nname: Mine\nsteps:\n  - {id: a, agent: citer, input: x}\n",
    )
    .unwrap();
    world.store.rescan_all(ChangedBy::Watcher);
    let err = world
        .installer
        .uninstall(InstallKind::Pack, "research-pack")
        .unwrap_err();
    assert!(err.contains("workflow 'mine'"), "{err}");

    fs::remove_file(&mine).unwrap();
    world.store.rescan_all(ChangedBy::Watcher);
    let removed = world
        .installer
        .uninstall(InstallKind::Pack, "research-pack")
        .unwrap();
    assert_eq!(removed, ["summarizer", "citer"]);
    assert!(!world.layout().pack_dir("research-pack").exists());
    assert!(world.projected("agents").is_empty());
    assert!(world.projected("workflows").is_empty());
    assert!(!world.layout().installed_path().exists());
}

// -- Through the app (registries, the watcher) ----------------------------------

fn file_url(dir: &Path) -> String {
    let path = dir.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn write_index(dir: &Path, agents: serde_json::Value) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("index.json"),
        serde_json::json!({ "schema_version": 1, "agents": agents }).to_string(),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_latest_version_across_registries_is_the_one_installed() {
    use crate::services::domain::settings::{RegistryConfig, RegistryKind};
    use crate::services::marketplace::{self, MarketplaceFilter};
    let state = crate::services::AppState::for_tests(Vec::new());
    let root = state.store().layout().root.clone();
    fs::create_dir_all(&root).unwrap();
    git2::Repository::init(&root).unwrap();
    state.store().load().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let old = tmp.path().join("old-source");
    commit(&old, &agent_files("old"), "v1.0.0");
    let new = tmp.path().join("new-source");
    commit(&new, &agent_files("new"), "v2.0.0");
    let entry = |repo: &Path, version: &str| {
        serde_json::json!([{
            "id": "arxiv", "name": "ArXiv", "publisher": "alice",
            "repo": repo.display().to_string(), "path": "agents/arxiv",
            "latest_version": version,
        }])
    };
    let (first, second) = (tmp.path().join("registry-a"), tmp.path().join("registry-b"));
    write_index(&first, entry(&old, "1.0.0"));
    write_index(&second, entry(&new, "2.0.0"));
    marketplace::save_registries(
        &state,
        vec![
            RegistryConfig::new(&file_url(&first), RegistryKind::Public, None).unwrap(),
            RegistryConfig::new(&file_url(&second), RegistryKind::Community, None).unwrap(),
        ],
    )
    .await
    .unwrap();

    let listed = marketplace::list_available_agents(&state, &MarketplaceFilter::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].latest_version, "2.0.0");

    let info = marketplace::install_agent(&state, "arxiv").await.unwrap();
    assert_eq!(info.version, "2.0.0");
    let skill = state
        .store()
        .layout()
        .vendored_agent_dir("alice", "arxiv")
        .join("skill.md");
    assert!(fs::read_to_string(skill).unwrap().ends_with("new"));
    let listed = marketplace::list_available_agents(
        &state,
        &MarketplaceFilter {
            installed_only: true,
            query: Some("arx".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(listed[0].installed_version.as_deref(), Some("2.0.0"));
    let infos = state.store().agent_infos();
    assert_eq!(
        infos[0].origin,
        crate::services::agents::AgentOrigin::Publisher {
            publisher: "alice".into()
        }
    );
    // It can't run until the user approves what it requests
    let refused = crate::services::chat::send_message(
        &state,
        "arxiv".into(),
        "hello".into(),
        Vec::new(),
        None,
    )
    .await
    .unwrap_err();
    assert!(refused.contains("needs your approval"), "{refused}");
    let consent = marketplace::get_agent_consent(&state, "arxiv")
        .await
        .unwrap();
    assert!(consent.required && !consent.satisfied);
    let categories = consent.categories.iter().map(|c| c.key.clone()).collect();
    let consent = marketplace::grant_agent_consent(&state, "arxiv", categories)
        .await
        .unwrap();
    assert!(consent.satisfied);
    // Deleting it like a local agent is refused; uninstall works
    assert!(crate::services::agents::delete_agent(&state, "arxiv")
        .await
        .unwrap_err()
        .contains("marketplace"));
    marketplace::uninstall(&state, "agent", "arxiv")
        .await
        .unwrap();
    assert!(state.store().agent_infos().is_empty());
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watcher_picks_up_a_manually_cloned_agent_folder() {
    let state = crate::services::AppState::for_tests(Vec::new());
    let layout = state.store().layout().clone();
    fs::create_dir_all(&layout.agents_dir).unwrap();
    state.store().load().unwrap();
    let _watcher = crate::services::watcher::spawn(
        state.store().clone(),
        state.bus().clone(),
        state.live().feed().clone(),
        state.runtime(),
    )
    .unwrap();

    // As if `git clone`d by hand, no install
    let dir = layout.vendored_agent_dir("bob", "test-writer");
    fs::create_dir_all(dir.join("workflows")).unwrap();
    fs::write(
        dir.join("skill.md"),
        "---\nname: Test Writer\n---\nWrite tests.",
    )
    .unwrap();
    for _ in 0..400 {
        if state.store().display_name("test-writer") == "Test Writer" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(state.store().display_name("test-writer"), "Test Writer");
    let conn = state.db().conn().unwrap();
    let name: String = conn
        .query_row(
            "SELECT name FROM agents WHERE id = 'test-writer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Test Writer");
    state.shutdown();
}

/// The published starter packs install side by side from the live default
/// registry (GitHub), exactly as the app does it. Needs the network:
/// `cargo test --lib live_starter_packs -- --ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fetches github.com/vaultdbai/nandi-registry"]
async fn live_starter_packs_install() {
    use super::registry::{Registry, RegistryClient};
    use crate::services::domain::settings::{RegistryKind, DEFAULT_REGISTRY};
    let world = World::new();
    let registry = Registry {
        url: DEFAULT_REGISTRY.to_string(),
        kind: RegistryKind::Public,
        auth: None,
    };
    let index = RegistryClient::new(world.path("cache"))
        .fetch_index(&registry, None, true)
        .await
        .unwrap();
    // The first-party pack is listed; the rest of the counts follow the index
    assert!(index.packs.iter().any(|p| p.id == "dev-pack"));
    // Every published release is signed by the publisher's key
    let client = RegistryClient::new(world.path("cache"));
    let publisher: super::manifest::PublisherFile = serde_json::from_slice(
        &client
            .fetch_file(&registry, "publishers/vaultdbai.json", None)
            .await
            .unwrap(),
    )
    .unwrap();
    let key = publisher.public_key.expect("vaultdbai signs releases");
    for pack in &index.packs {
        let ok = super::signature::verify(
            &key,
            pack.signature.as_deref().unwrap(),
            "pack",
            &pack.id,
            &pack.latest_version,
            pack.commit.as_deref().unwrap(),
        );
        assert!(ok, "pack {} signature", pack.id);
    }
    for agent in &index.agents {
        let ok = super::signature::verify(
            &key,
            agent.signature.as_deref().unwrap(),
            "agent",
            &agent.id,
            &agent.latest_version,
            agent.commit.as_deref().unwrap(),
        );
        assert!(ok, "agent {} signature", agent.id);
    }
    let installer = Installer::new(world.store.clone(), world.path("scratch"));
    let packs = index.packs.clone();
    let installed = tokio::task::spawn_blocking(move || {
        packs
            .iter()
            .map(|pack| {
                installer
                    .install_pack(pack, None, &mut |_| {})
                    .unwrap_or_else(|e| panic!("{}: {e}", pack.id))
                    .agents
                    .len()
            })
            .sum::<usize>()
    })
    .await
    .unwrap();
    assert_eq!(installed, index.agents.len());
    assert_eq!(world.projected("agents").len(), index.agents.len());
    let workflows: usize = index.packs.iter().map(|p| p.workflows.len()).sum();
    assert_eq!(world.projected("workflows").len(), workflows);
    let problems = world.store.read(|repo| repo.problems());
    assert!(problems.is_empty(), "{problems:?}");
}

/// A signed entry verifies against its publisher's key and installs as
/// verified; an unsigned one doesn't verify; a tag moved after signing is
/// refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signatures_verify_and_pin_the_release_commit() {
    use super::signature;
    use crate::services::domain::settings::{RegistryConfig, RegistryKind};
    use crate::services::marketplace::{self, MarketplaceFilter};
    let state = crate::services::AppState::for_tests(Vec::new());
    let root = state.store().layout().root.clone();
    fs::create_dir_all(&root).unwrap();
    git2::Repository::init(&root).unwrap();
    state.store().load().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let signed_repo = tmp.path().join("signed");
    let commit_signed = commit(&signed_repo, &agent_files("signed"), "v1.0.0");
    let moved_repo = tmp.path().join("moved");
    let moved_files = vec![(
        "agents/mover/skill.md".to_string(),
        "---\nname: Mover\n---\n".to_string(),
    )];
    let commit_moved = commit(&moved_repo, &moved_files, "v1.0.0");

    let (secret, public) = signature::generate();
    let sign = |id: &str, sha: &str| signature::sign(&secret, "agent", id, "1.0.0", sha).unwrap();
    let entry = |id: &str, repo: &Path, sha: Option<&str>, sig: Option<String>| {
        let mut value = serde_json::json!({
            "id": id, "name": id, "publisher": "alice",
            "repo": repo.display().to_string(), "path": format!("agents/{id}"),
            "latest_version": "1.0.0",
        });
        if let (Some(sha), Some(sig)) = (sha, sig) {
            value["commit"] = sha.into();
            value["signature"] = sig.into();
        }
        value
    };
    let registry = tmp.path().join("registry");
    write_index(
        &registry,
        serde_json::json!([
            entry(
                "arxiv",
                &signed_repo,
                Some(&commit_signed),
                Some(sign("arxiv", &commit_signed))
            ),
            entry(
                "mover",
                &moved_repo,
                Some(&commit_moved),
                Some(sign("mover", &commit_moved))
            ),
            entry("other", &signed_repo, None, None),
        ]),
    );
    fs::create_dir_all(registry.join("publishers")).unwrap();
    fs::write(
        registry.join("publishers/alice.json"),
        serde_json::json!({ "github_handle": "alice", "public_key": public }).to_string(),
    )
    .unwrap();
    marketplace::save_registries(
        &state,
        vec![RegistryConfig::new(&file_url(&registry), RegistryKind::Personal, None).unwrap()],
    )
    .await
    .unwrap();

    let listed = marketplace::list_available_agents(&state, &MarketplaceFilter::default())
        .await
        .unwrap();
    let verified = |id: &str| listed.iter().find(|a| a.id == id).unwrap().verified;
    assert!(verified("arxiv") && verified("mover"));
    assert!(!verified("other"));
    let only_verified = marketplace::list_available_agents(
        &state,
        &MarketplaceFilter {
            verified_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(only_verified.len(), 2);

    marketplace::install_agent(&state, "arxiv").await.unwrap();
    let installed = state.store().repos().vendor.load_installed().unwrap();
    assert!(installed.agent("arxiv").unwrap().signature_verified);

    // The publisher's tag is moved to another commit after signing
    commit(
        &moved_repo,
        &[(
            "agents/mover/skill.md".to_string(),
            "---\nname: Evil\n---\n".to_string(),
        )],
        "v1.0.0",
    );
    let refused = marketplace::install_agent(&state, "mover")
        .await
        .unwrap_err();
    assert!(refused.contains("tampered"), "{refused}");
    assert!(state.store().repos().agents.locate("mover").is_none());
    state.shutdown();
}

#[test]
fn verified_official_entries_come_first() {
    // (id, tags, verified): an unverified "official" claim doesn't jump ahead
    let mut items = vec![
        ("community", vec!["engineering".to_string()], true),
        ("claims-official", vec!["official".to_string()], false),
        (
            "dev-pack",
            vec!["Official".to_string(), "free".to_string()],
            true,
        ),
        ("other", vec![], true),
    ];
    super::official_first(&mut items, |i| &i.1, |i| i.2);
    let ids: Vec<_> = items.iter().map(|i| i.0).collect();
    assert_eq!(ids, ["dev-pack", "community", "claims-official", "other"]);
}

// -- From a folder or .zip on this computer -------------------------------------

/// `files` written under `dir`
fn folder(dir: &Path, files: &[(String, String)]) -> PathBuf {
    for (path, content) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    dir.to_path_buf()
}

fn local(path: &Path) -> super::local::LocalPackSource {
    super::local::LocalPackSource::open(path).unwrap()
}

#[test]
fn installs_a_pack_from_a_local_folder_as_an_unverified_local_install() {
    let world = World::new();
    let dir = folder(&world.path("my pack"), &pack_files("1.0.0", "Summarize."));

    let info = world.installer.install_local_pack(&local(&dir)).unwrap();
    assert_eq!(info.id, "research-pack");
    assert_eq!(info.agents, ["summarizer", "citer"]);
    assert_eq!(world.projected("agents"), ["citer", "summarizer"]);
    assert_eq!(world.projected("workflows"), ["citer-daily", "review"]);
    let pack_dir = world.layout().pack_dir("research-pack");
    assert!(!pack_dir.join("scripts").exists(), "only allowlisted files");

    let installed = world.store.repos().vendor.load_installed().unwrap();
    let entry = installed.pack("research-pack").unwrap();
    assert!(entry.is_local());
    assert!(entry.source.starts_with("local:") && entry.source.ends_with("my pack"));
    assert_eq!(entry.git_ref, "local");
    assert_eq!(entry.registry, None);
    assert!(!entry.signature_verified);
    assert_eq!(entry.commit, entry.tree_hash);
    assert_eq!(info.commit, entry.commit);

    // Again: refused, the way to reinstall named
    let err = world
        .installer
        .install_local_pack(&local(&dir))
        .unwrap_err();
    assert!(
        err.contains("already installed") && err.contains("Uninstall"),
        "{err}"
    );

    // No source to check: neither an update nor a failure
    let (updates, failures) = world.installer.check_for_updates(&|_| None).unwrap();
    assert!(updates.is_empty() && failures.is_empty(), "{failures:?}");
    let err = world
        .installer
        .apply_update("research-pack", "v2.0.0", None, None, &mut no_progress())
        .unwrap_err();
    assert!(err.contains("folder on this computer"), "{err}");

    // Uninstalls like any pack
    let removed = world
        .installer
        .uninstall(InstallKind::Pack, "research-pack")
        .unwrap();
    assert_eq!(removed, ["summarizer", "citer"]);
    assert!(!pack_dir.exists());
    assert!(world.projected("agents").is_empty());
}

#[test]
fn installs_a_pack_from_a_zip_of_its_folder() {
    let world = World::new();
    let files = pack_files("1.2.0", "x");
    let wrapped: Vec<(String, &str)> = files
        .iter()
        .map(|(p, c)| (format!("research-pack-main/{p}"), c.as_str()))
        .collect();
    let named: Vec<(&str, &str)> = wrapped.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    let zip = world.path("research-pack.zip");
    super::local::tests::zip_of(&zip, &named);

    let info = world.installer.install_local_pack(&local(&zip)).unwrap();
    assert_eq!(
        (info.id.as_str(), info.version.as_str()),
        ("research-pack", "1.2.0")
    );
    assert_eq!(world.projected("agents"), ["citer", "summarizer"]);
}

#[test]
fn local_packs_get_the_same_checks_as_registry_packs() {
    let world = World::new();

    // Not a pack
    let empty = folder(&world.path("empty"), &[("README.md".into(), "hi".into())]);
    let err = world
        .installer
        .install_local_pack(&local(&empty))
        .unwrap_err();
    assert!(err.contains("no .nandi/manifest.yaml"), "{err}");

    // An agent that doesn't parse
    let mut files = pack_files("1.0.0", "x");
    files[2].1 = "---\nname: [unclosed\n---\n".into();
    let broken = folder(&world.path("broken"), &files);
    assert!(world.installer.install_local_pack(&local(&broken)).is_err());
    assert!(world.projected("agents").is_empty());

    // Needs a newer Nandi
    let mut files = pack_files("1.0.0", "x");
    files[0].1.push_str("min_nandi_version: 99.0.0\n");
    let future = folder(&world.path("future"), &files);
    let err = world
        .installer
        .install_local_pack(&local(&future))
        .unwrap_err();
    assert!(err.contains("99.0.0"), "{err}");

    // An ID that's taken
    let mine = world.layout().skill_path("summarizer");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "---\nname: Mine\n---\n").unwrap();
    world.store.rescan_all(ChangedBy::Watcher);
    let ok = folder(&world.path("ok"), &pack_files("1.0.0", "x"));
    let err = world.installer.install_local_pack(&local(&ok)).unwrap_err();
    assert!(err.contains("agent 'summarizer'"), "{err}");

    // A .zip reaching outside the pack
    let zip = world.path("evil.zip");
    super::local::tests::zip_of(
        &zip,
        &[
            (
                ".nandi/manifest.yaml",
                "id: evil\nname: Evil\nversion: 1.0.0\n",
            ),
            ("../x", "y"),
        ],
    );
    let err = world
        .installer
        .install_local_pack(&local(&zip))
        .unwrap_err();
    assert!(err.contains("outside the pack"), "{err}");
}

#[test]
fn inspecting_a_local_pack_describes_it_for_the_consent_dialog() {
    let world = World::new();
    let dir = folder(&world.path("pack"), &pack_files("1.0.0", "x"));
    let manifest = world.installer.inspect_local_pack(&local(&dir)).unwrap();
    assert_eq!(manifest.id, "research-pack");
    assert_eq!(manifest.name, "Research Pack");
    assert_eq!(manifest.registry, "local");
    assert!(!manifest.verified && manifest.compatible);
    assert_eq!(manifest.installed_version, None);
    let agents: Vec<&str> = manifest.agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(agents, ["Summarizer", "Citer"]);
    assert_eq!(manifest.workflows, ["review"]);
    // The summarizer lists its tools; the citer gets every tool
    assert_eq!(manifest.capabilities.tools, ["read_file", "*"]);
    // Nothing was installed
    assert!(world.projected("agents").is_empty());

    world.installer.install_local_pack(&local(&dir)).unwrap();
    let again = world.installer.inspect_local_pack(&local(&dir)).unwrap();
    assert_eq!(again.installed_version.as_deref(), Some("1.0.0"));
}

#[test]
fn installed_local_packs_are_listed_from_their_vendored_copy() {
    let world = World::new();
    assert!(world.installer.installed_local_packs().unwrap().is_empty());
    let dir = folder(&world.path("my pack"), &pack_files("1.0.0", "x"));
    let info = world.installer.install_local_pack(&local(&dir)).unwrap();
    // The folder it came from can go away; the vendored copy describes it
    fs::remove_dir_all(&dir).unwrap();

    let listed = world.installer.installed_local_packs().unwrap();
    assert_eq!(listed.len(), 1);
    let pack = &listed[0];
    assert_eq!(pack.id, "research-pack");
    assert_eq!(pack.name, "Research Pack");
    assert_eq!(pack.registry, "local");
    assert!(pack.repo.starts_with("local:") && pack.repo.ends_with("my pack"));
    assert_eq!(pack.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(pack.commit.as_deref(), Some(info.commit.as_str()));
    assert!(!pack.verified);
    let agents: Vec<&str> = pack.agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(agents, ["summarizer", "citer"]);

    world
        .installer
        .uninstall(InstallKind::Pack, "research-pack")
        .unwrap();
    assert!(world.installer.installed_local_packs().unwrap().is_empty());
}

#[test]
fn the_offline_fixture_pack_installs() {
    let world = World::new();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-fixtures/offline-pack");
    let info = world
        .installer
        .install_local_pack(&local(&fixture))
        .unwrap();
    assert_eq!(info.id, "offline-pack");
    assert_eq!(info.agents, ["offline-helper"]);
    assert_eq!(info.workflows, ["offline-hello"]);
    assert_eq!(world.projected("workflows"), ["offline-hello"]);
}
