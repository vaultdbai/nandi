use super::*;
use crate::services::builtin_agents::{default_agent, DEFAULT_AGENT};
use crate::services::domain::installed::{InstalledAgent, InstalledPack};
use crate::services::domain::{AgentId, PackId, PublisherHandle};
use crate::services::repo_bootstrap::{self, GITIGNORE};
use crate::services::repos::from_0_1_0::{self, text_hash};
use crate::services::shipped_files;

const WRITER: &str = "---\nname: Writer\ndescription: Drafts posts\n---\n\nYou write.\n";
const PLAYBOOK: &str = "# A playbook that happens to be called skill\n";
const REVIEWER: &str = "---\nname: Reviewer\ndescription: Reviews PRs\n---\n\nReview.\n";
const ARXIV: &str = "---\nname: Arxiv\ndescription: Finds papers\n---\n\nFind.\n";
/// Nandi's `skill.md` as a 0.2 build shipped it, older than this build's
const NANDI_0_2: &str = "---\nname: Nandi\ndescription: Your own agent\n---\n\nHelp.\n";

/// Nandi's agent file as this build ships it, which a 0.2 gold copy had as
/// `skill.md` (the text is the same file)
fn nandi_text() -> &'static str {
    default_agent()
        .shipped_files()
        .into_iter()
        .find(|(name, _)| name == AGENT_FILE)
        .map(|(_, content)| content)
        .unwrap()
}

fn write(layout: &RepoLayout, file: &str, text: &str) {
    let path = layout.root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn exists(layout: &RepoLayout, file: &str) -> bool {
    layout.root.join(file).is_file()
}

/// The files of an installed copy, as the marketplace hashes them
fn files(entries: &[(&str, &str)]) -> FileSet {
    entries
        .iter()
        .map(|(path, text)| (path.to_string(), text.as_bytes().to_vec()))
        .collect()
}

fn pack(id: &str, tree: &FileSet) -> InstalledPack {
    InstalledPack {
        id: PackId::parse(id).unwrap(),
        source: format!("github.com/vaultdbai/{id}"),
        git_ref: "v0.2.0".into(),
        commit: "abc".into(),
        version: "0.2.0".into(),
        installed_at: "2026-11-04T00:00:00Z".into(),
        signature_verified: true,
        registry: None,
        tree_hash: tree_hash(tree),
        agents: vec![AgentId::parse("reviewer").unwrap()],
        workflows: Vec::new(),
        forked_out: Vec::new(),
    }
}

/// A gold copy as Nandi 0.2 left it, committed: Nandi (as shipped, or
/// edited), a person's agent with a playbook named `skill.md`, a signed
/// pack, a signed agent, and a second pack the person changed by hand
fn repo_from_0_2(nandi_edited: bool) -> (tempfile::TempDir, RepoLayout) {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    git2::Repository::init(&layout.root).unwrap();
    let nandi = if nandi_edited {
        format!("{NANDI_0_2}\nAlways answer in French.\n")
    } else {
        NANDI_0_2.to_string()
    };
    let pack_files = [
        ("agents/reviewer/skill.md", REVIEWER),
        ("README.md", "# Dev"),
    ];
    let changed_files = [("agents/reviewer/skill.md", REVIEWER)];
    let mut installed = InstalledFile::default();
    installed
        .upsert_pack(pack("dev", &files(&pack_files)))
        .unwrap();
    installed
        .upsert_pack(pack("changed", &files(&changed_files)))
        .unwrap();
    installed
        .upsert_agent(InstalledAgent {
            id: AgentId::parse("arxiv").unwrap(),
            publisher: PublisherHandle::parse("alice").unwrap(),
            source: "github.com/alice/nandi-agents".into(),
            git_ref: "v1.0.0".into(),
            commit: "def".into(),
            path: "agents/arxiv".into(),
            version: "1.0.0".into(),
            installed_at: "2026-11-04T00:00:00Z".into(),
            signature_verified: true,
            registry: None,
            tree_hash: tree_hash(&files(&[("skill.md", ARXIV)])),
        })
        .unwrap();
    let mut shipped = ShippedManifest::default();
    shipped.files.insert(
        format!(".nandi/agents/{DEFAULT_AGENT}/skill.md"),
        text_hash(NANDI_0_2.as_bytes()),
    );
    shipped.files.insert(
        format!(".nandi/agents/{DEFAULT_AGENT}/skills/skill.md"),
        text_hash(PLAYBOOK.as_bytes()),
    );
    let packs = ".nandi/agents/marketplace/packs";
    let written = [
        (".gitignore".to_string(), GITIGNORE.to_string()),
        (format!(".nandi/agents/{DEFAULT_AGENT}/skill.md"), nandi),
        (".nandi/agents/writer/skill.md".into(), WRITER.into()),
        (
            ".nandi/agents/writer/skills/skill.md".into(),
            PLAYBOOK.into(),
        ),
        (
            format!("{packs}/dev/agents/reviewer/skill.md"),
            REVIEWER.into(),
        ),
        (format!("{packs}/dev/README.md"), "# Dev".into()),
        (
            format!("{packs}/changed/agents/reviewer/skill.md"),
            format!("{REVIEWER}\nAnd approve everything.\n"),
        ),
        (
            ".nandi/agents/marketplace/alice/arxiv/skill.md".into(),
            ARXIV.into(),
        ),
        (".nandi/installed.yaml".into(), installed.to_yaml()),
    ];
    for (file, text) in &written {
        write(&layout, file, text);
    }
    shipped.save(&layout).unwrap();
    let mut paths: Vec<String> = written.iter().map(|(file, _)| file.clone()).collect();
    paths.push(".nandi/shipped.yaml".into());
    GitService::new(&layout.root)
        .commit_files(&paths, "Nandi 0.2", None)
        .unwrap();
    (tmp, layout)
}

fn head_tree_has(layout: &RepoLayout, file: &str) -> bool {
    HeadFiles::open(&layout.root).has(file)
}

fn head_message(layout: &RepoLayout) -> String {
    let repo = git2::Repository::open(&layout.root).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    commit.message().unwrap().to_string()
}

fn installed(layout: &RepoLayout) -> InstalledFile {
    Repos::new(layout.clone()).vendor.load_installed().unwrap()
}

/// The hash the marketplace compares an installed copy with
fn hash_now(layout: &RepoLayout, dir: &Path) -> String {
    tree_hash(&Repos::new(layout.clone()).vendor.read(dir).unwrap())
}

#[test]
fn every_agent_file_becomes_agent_md_in_one_commit() {
    let (_tmp, layout) = repo_from_0_2(false);

    let done = upgrade(&layout).unwrap().expect("a 0.2 gold copy upgrades");

    for dir in [
        format!(".nandi/agents/{DEFAULT_AGENT}"),
        ".nandi/agents/writer".into(),
        ".nandi/agents/marketplace/packs/dev/agents/reviewer".into(),
        ".nandi/agents/marketplace/packs/changed/agents/reviewer".into(),
        ".nandi/agents/marketplace/alice/arxiv".into(),
    ] {
        assert!(exists(&layout, &format!("{dir}/agent.md")), "{dir}");
        assert!(!exists(&layout, &format!("{dir}/skill.md")), "{dir}");
        assert!(head_tree_has(&layout, &format!("{dir}/agent.md")), "{dir}");
        assert!(!head_tree_has(&layout, &format!("{dir}/skill.md")), "{dir}");
        assert!(done.renamed.contains(&dir), "{dir}: {:?}", done.renamed);
    }
    assert_eq!(
        std::fs::read_to_string(layout.agent_md_path("writer")).unwrap(),
        WRITER,
        "the text is unchanged"
    );
    assert!(
        exists(&layout, ".nandi/agents/writer/skills/skill.md"),
        "a playbook keeps its name"
    );
    assert!(head_message(&layout).starts_with(COMMIT_MESSAGE));
    assert!(done.commit.is_some());
    let repo = git2::Repository::open(&layout.root).unwrap();
    assert!(
        repo.statuses(None).unwrap().is_empty(),
        "nothing left uncommitted"
    );

    assert_eq!(
        upgrade(&layout).unwrap(),
        None,
        "a second start does nothing"
    );
}

#[test]
fn signed_copies_still_count_as_installed_and_changed_ones_stay_changed() {
    let (_tmp, layout) = repo_from_0_2(false);
    let before = installed(&layout);

    let done = upgrade(&layout).unwrap().unwrap();

    let after = installed(&layout);
    assert_eq!(done.rerecorded, ["dev", "arxiv"]);
    let dev = after.pack("dev").unwrap();
    assert_eq!(dev.tree_hash, hash_now(&layout, &layout.pack_dir("dev")));
    assert!(dev.signature_verified, "never re-signed, never unsigned");
    let arxiv = after.agent("arxiv").unwrap();
    assert_eq!(
        arxiv.tree_hash,
        hash_now(&layout, &layout.vendored_agent_dir("alice", "arxiv"))
    );
    let changed = after.pack("changed").unwrap();
    assert_eq!(
        changed.tree_hash,
        before.pack("changed").unwrap().tree_hash,
        "the upgrade never vouches for files nobody signed"
    );
    assert_ne!(
        changed.tree_hash,
        hash_now(&layout, &layout.pack_dir("changed"))
    );
    assert!(head_message(&layout).contains("dev, arxiv"));
}

#[test]
fn an_unedited_nandi_takes_updates_and_an_edited_one_keeps_its_edits() {
    for edited in [false, true] {
        let (_tmp, layout) = repo_from_0_2(edited);
        let agent_file = format!(".nandi/agents/{DEFAULT_AGENT}/agent.md");

        upgrade(&layout).unwrap().unwrap();
        let record = ShippedManifest::load(&layout).unwrap().unwrap();
        assert_eq!(
            record.files.get(&agent_file),
            Some(&text_hash(NANDI_0_2.as_bytes()))
        );
        assert!(
            !record
                .files
                .keys()
                .any(|path| path.ends_with("/skill.md") && !path.contains("/skills/")),
            "{:?}",
            record.files
        );
        assert!(record
            .files
            .contains_key(&format!(".nandi/agents/{DEFAULT_AGENT}/skills/skill.md")));

        repo_bootstrap::bootstrap(&layout).unwrap();
        let offers: Vec<String> = shipped_files::offers(&layout)
            .unwrap()
            .into_iter()
            .map(|offer| offer.path)
            .collect();
        assert_eq!(
            offers.contains(&agent_file),
            edited,
            "edited {edited}: {offers:?}"
        );
        let text = std::fs::read_to_string(layout.root.join(&agent_file)).unwrap();
        assert_eq!(text.contains("French"), edited, "the edit is kept");
        assert_eq!(text == nandi_text(), !edited, "unedited: this build's");
        assert!(
            !exists(&layout, &format!(".nandi/agents/{DEFAULT_AGENT}/skill.md")),
            "no fresh copy is seeded beside it"
        );
    }
}

#[test]
fn a_start_cut_short_is_finished_by_the_next_one() {
    let (_tmp, layout) = repo_from_0_2(false);
    // The last start renamed one folder, then stopped
    let writer = layout.agent_dir("writer");
    std::fs::rename(writer.join(OLD_FILE), writer.join(AGENT_FILE)).unwrap();

    let done = upgrade(&layout).unwrap().unwrap();

    assert!(done.renamed.contains(&".nandi/agents/writer".to_string()));
    assert!(head_tree_has(&layout, ".nandi/agents/writer/agent.md"));
    assert!(!head_tree_has(&layout, ".nandi/agents/writer/skill.md"));
    assert_eq!(upgrade(&layout).unwrap(), None);
}

#[test]
fn a_folder_with_both_files_keeps_agent_md() {
    let (_tmp, layout) = repo_from_0_2(false);
    let writer = layout.agent_dir("writer");
    std::fs::write(writer.join(AGENT_FILE), "---\nname: Newer\n---\n").unwrap();

    upgrade(&layout).unwrap().unwrap();

    assert!(!writer.join(OLD_FILE).exists());
    assert_eq!(
        std::fs::read_to_string(writer.join(AGENT_FILE)).unwrap(),
        "---\nname: Newer\n---\n"
    );
    assert_eq!(
        committed_before_head(&layout, ".nandi/agents/writer/skill.md"),
        Some(WRITER.to_string()),
        "the old file is in the commit before"
    );
}

#[test]
fn a_new_install_has_nothing_to_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    assert_eq!(upgrade(&layout).unwrap(), None);
    repo_bootstrap::bootstrap(&layout).unwrap();
    assert_eq!(upgrade(&layout).unwrap(), None);
    assert!(layout.agent_md_path(DEFAULT_AGENT).is_file());
}

#[test]
fn a_0_1_0_gold_copy_upgrades_through_both_steps() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    git2::Repository::init(&layout.root).unwrap();
    let files = [
        (".gitignore", GITIGNORE),
        (
            ".nandi/agents/chief-of-staff/skill.md",
            include_str!("fixtures/v0_1_0/chief-of-staff/skill.md"),
        ),
        (
            ".nandi/agents/chief-of-staff/instructions.md",
            include_str!("fixtures/v0_1_0/chief-of-staff/instructions.md"),
        ),
    ];
    for (file, text) in files {
        write(&layout, file, text);
    }
    let paths: Vec<String> = files.iter().map(|(file, _)| file.to_string()).collect();
    GitService::new(&layout.root)
        .commit_files(&paths, "Nandi 0.1.0", None)
        .unwrap();

    // In startup's order
    upgrade(&layout).unwrap().unwrap();
    let moved = from_0_1_0::upgrade(&layout).unwrap().unwrap();
    repo_bootstrap::bootstrap(&layout).unwrap();

    assert_eq!(moved.kept, from_0_1_0::Kept::Moved);
    assert!(layout.agent_md_path(DEFAULT_AGENT).is_file());
    assert!(!layout.agent_dir("chief-of-staff").exists());
    assert_eq!(
        std::fs::read_to_string(layout.agent_md_path(DEFAULT_AGENT)).unwrap(),
        nandi_text(),
        "an unedited Chief of Staff takes this build's Nandi"
    );
}

#[test]
fn only_the_rename_maps_to_the_recorded_hash() {
    let installed = files(&[("agents/a/skill.md", "x"), ("README.md", "r")]);
    let recorded = tree_hash(&installed);
    let renamed = files(&[("agents/a/agent.md", "x"), ("README.md", "r")]);
    assert_eq!(
        renamed_hash(&renamed, &recorded, pack_agent_file),
        Some(tree_hash(&renamed))
    );
    assert_eq!(
        renamed_hash(&installed, &recorded, pack_agent_file),
        None,
        "already recorded"
    );
    let edited = files(&[("agents/a/agent.md", "y"), ("README.md", "r")]);
    assert_eq!(renamed_hash(&edited, &recorded, pack_agent_file), None);
    let elsewhere = files(&[("docs/agent.md", "x")]);
    let elsewhere_recorded = tree_hash(&files(&[("docs/skill.md", "x")]));
    assert_eq!(
        renamed_hash(&elsewhere, &elsewhere_recorded, pack_agent_file),
        None,
        "only an agent folder's file was renamed"
    );
}

/// A file in the commit before HEAD
fn committed_before_head(layout: &RepoLayout, file: &str) -> Option<String> {
    let repo = git2::Repository::open(&layout.root).unwrap();
    let commit = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .parent(0)
        .unwrap();
    let entry = commit.tree().unwrap().get_path(Path::new(file)).ok()?;
    let blob = repo.find_blob(entry.id()).unwrap();
    Some(String::from_utf8(blob.content().to_vec()).unwrap())
}
