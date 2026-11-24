use super::*;
use crate::services::builtin_agents;
use crate::services::repo_bootstrap::GITIGNORE;
use crate::services::repos::layout::{AGENT_FILE, INSTRUCTIONS_FILE};
use crate::services::repos::Repos;

/// Nandi's files as 0.1.0 seeded them into the Chief of Staff's folder
const SKILL_0_1_0: &str = include_str!("repos/fixtures/v0_1_0/chief-of-staff/skill.md");
const INSTRUCTIONS_0_1_0: &str =
    include_str!("repos/fixtures/v0_1_0/chief-of-staff/instructions.md");

/// A role the person wrote in place of the template
const MY_ROLE: &str = "# My role\n\n- **Name and title:** Ada, finance lead\n";

/// This build's version of a file in Nandi's folder (`skills/occasions.md`)
fn today(name: &str) -> &'static str {
    default_agent()
        .shipped_files()
        .into_iter()
        .find(|(file, _)| file == name)
        .map(|(_, content)| content)
        .unwrap()
}

/// What the next build ships of a file in Nandi's folder: today's version
/// with `added` at the end
fn next(name: &str, added: &str) -> &'static str {
    Box::leak(format!("{}{added}", today(name)).into_boxed_str())
}

fn next_skill() -> &'static str {
    next(AGENT_FILE, "\nCheck the calendar before you plan a day.\n")
}

fn next_instructions() -> &'static str {
    next(INSTRUCTIONS_FILE, "\n- **Working hours:**\n")
}

const NEW_TRIGGER: &str = "\
id: evening-review
name: Evening review
enabled: true
source:
  type: cron
  config:
    schedule: \"0 18 * * 1-5\"
target:
  type: agent
  id: nandi
";

fn hash(text: &str) -> String {
    text_hash(text.as_bytes())
}

// --- What to do with one file ------------------------------------------

#[test]
fn decides_by_what_nandi_wrote_what_is_there_and_what_ships() {
    let (v1, v2, mine) = (hash("v1"), hash("v2"), hash("mine"));
    let (v1, v2, mine) = (v1.as_str(), v2.as_str(), mine.as_str());
    // (case, written, on disk, shipped, kept, step)
    let cases = [
        (
            "unedited, changed",
            Some(v1),
            Some(v1),
            v2,
            None,
            Step::Replace,
        ),
        (
            "unedited, unchanged",
            Some(v1),
            Some(v1),
            v1,
            None,
            Step::Record,
        ),
        (
            "edited, changed",
            Some(v1),
            Some(mine),
            v2,
            None,
            Step::Offer,
        ),
        (
            "edited, unchanged",
            Some(v1),
            Some(mine),
            v1,
            None,
            Step::Leave,
        ),
        (
            "edited, kept mine",
            Some(v1),
            Some(mine),
            v2,
            Some(v2),
            Step::Leave,
        ),
        (
            "edited, kept an older one",
            Some(v1),
            Some(mine),
            v2,
            Some(v1),
            Step::Offer,
        ),
        ("deleted", Some(v1), None, v2, None, Step::Leave),
        ("new", None, None, v2, None, Step::Add),
        (
            "the person's own file",
            None,
            Some(mine),
            v2,
            None,
            Step::Offer,
        ),
        (
            "already the new version",
            Some(v1),
            Some(v2),
            v2,
            None,
            Step::Record,
        ),
        (
            "never recorded, the same",
            None,
            Some(v2),
            v2,
            None,
            Step::Record,
        ),
    ];
    for (case, written, on_disk, shipped, kept, step) in cases {
        assert_eq!(decide(written, on_disk, shipped, kept), step, "{case}");
    }
}

// --- A gold copy -----------------------------------------------------------

/// A gold copy as this build seeds it: committed, with its record
fn gold_copy() -> (tempfile::TempDir, RepoStore, GitService) {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    crate::services::repo_bootstrap::bootstrap(&layout).unwrap();
    let git = GitService::new(&layout.root);
    let store = RepoStore::sqlite(Repos::new(layout), crate::db::Db::in_memory());
    store.load().unwrap();
    (tmp, store, git)
}

/// A gold copy as Nandi 0.1.0 left it once the Chief of Staff is Nandi:
/// its two files (the role as `role`), no playbooks, triggers or record
fn gold_copy_from_0_1_0(role: &str) -> (tempfile::TempDir, RepoLayout) {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    git2::Repository::init(&layout.root).unwrap();
    let files = [
        (".gitignore".to_string(), GITIGNORE),
        (nandi(AGENT_FILE), SKILL_0_1_0),
        (nandi(INSTRUCTIONS_FILE), role),
    ];
    for (file, text) in &files {
        write(&layout, file, text);
    }
    let paths: Vec<String> = files.iter().map(|(file, _)| file.clone()).collect();
    GitService::new(&layout.root)
        .commit_files(&paths, "Nandi 0.1.0", None)
        .unwrap();
    (tmp, layout)
}

/// `name` in Nandi's folder, relative to the root
fn nandi(name: &str) -> String {
    format!(".nandi/agents/{DEFAULT_AGENT}/{name}")
}

fn write(layout: &RepoLayout, file: &str, text: &str) {
    let path = layout.root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(layout: &RepoLayout, file: &str) -> Option<String> {
    std::fs::read_to_string(layout.root.join(file)).ok()
}

fn agent_file(path: String, content: &'static str) -> ShippedFile {
    ShippedFile {
        path,
        content,
        kind: EntityKind::Agent,
        id: DEFAULT_AGENT.to_string(),
    }
}

/// The next build's files: agent.md, instructions.md and the occasions
/// playbook changed, and a new trigger
fn next_build() -> Vec<ShippedFile> {
    vec![
        agent_file(nandi(AGENT_FILE), next_skill()),
        agent_file(nandi(INSTRUCTIONS_FILE), next_instructions()),
        agent_file(
            nandi("skills/occasions.md"),
            next("skills/occasions.md", "\nAsk before you buy a gift.\n"),
        ),
        ShippedFile {
            path: ".nandi/triggers/evening-review.yaml".to_string(),
            content: NEW_TRIGGER,
            kind: EntityKind::Trigger,
            id: "evening-review".to_string(),
        },
    ]
}

fn head(layout: &RepoLayout) -> (git2::Repository, git2::Oid) {
    let repo = git2::Repository::open(&layout.root).unwrap();
    let oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    (repo, oid)
}

/// The text of `path` in the commit at HEAD
fn committed(layout: &RepoLayout, path: &str) -> Option<String> {
    let (repo, oid) = head(layout);
    let tree = repo.find_commit(oid).unwrap().tree().unwrap();
    let entry = tree.get_path(std::path::Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).unwrap();
    Some(String::from_utf8(blob.content().to_vec()).unwrap())
}

fn head_message(layout: &RepoLayout) -> String {
    let (repo, oid) = head(layout);
    let message = repo
        .find_commit(oid)
        .unwrap()
        .message()
        .unwrap()
        .to_string();
    message
}

fn manifest(layout: &RepoLayout) -> ShippedManifest {
    ShippedManifest::load(layout).unwrap().expect("recorded")
}

// --- Updating at startup ---------------------------------------------------

#[test]
fn a_fresh_install_records_every_shipped_file_in_its_first_commit() {
    let (_tmp, store, _git) = gold_copy();
    let layout = store.layout();
    let record = manifest(layout);
    for file in shipped_files(layout) {
        assert_eq!(
            record.files.get(&file.path),
            Some(&file.hash()),
            "{}",
            file.path
        );
    }
    assert!(record.kept.is_empty());
    assert!(committed(layout, ".nandi/shipped.yaml").is_some());
    assert_eq!(
        update(layout).unwrap(),
        Updated::default(),
        "nothing to do again"
    );
}

#[test]
fn an_update_replaces_unedited_files_keeps_edited_ones_and_adds_new_ones() {
    let (_tmp, store, _git) = gold_copy();
    let layout = store.layout();
    write(layout, &nandi(INSTRUCTIONS_FILE), MY_ROLE);
    std::fs::remove_file(layout.root.join(nandi("skills/occasions.md"))).unwrap();

    let updated = update_files(layout, &next_build()).unwrap();
    crate::services::repo_bootstrap::commit_shipped_update(layout, &updated).unwrap();

    // (file, what is there now)
    let expected = [
        ("unedited", nandi(AGENT_FILE), Some(next_skill())),
        ("edited", nandi(INSTRUCTIONS_FILE), Some(MY_ROLE)),
        ("deleted", nandi("skills/occasions.md"), None),
        (
            "new",
            ".nandi/triggers/evening-review.yaml".into(),
            Some(NEW_TRIGGER),
        ),
    ];
    for (case, path, text) in &expected {
        assert_eq!(read(layout, path).as_deref(), *text, "{case}: {path}");
    }
    assert_eq!(updated.replaced, [nandi(AGENT_FILE)]);
    assert_eq!(updated.added, [".nandi/triggers/evening-review.yaml"]);

    // Committed, saying what changed
    let message = head_message(layout);
    assert!(message.starts_with(UPDATE_COMMIT), "{message}");
    assert!(message.contains(&nandi(AGENT_FILE)), "{message}");
    assert_eq!(
        committed(layout, &nandi(AGENT_FILE)).as_deref(),
        Some(next_skill())
    );
    assert_eq!(
        committed(layout, ".nandi/triggers/evening-review.yaml").as_deref(),
        Some(NEW_TRIGGER)
    );

    // Only the edited file is on offer, and the next start changes nothing
    let offered: Vec<String> = offers_of(layout, &next_build())
        .unwrap()
        .into_iter()
        .map(|offer| offer.path)
        .collect();
    assert_eq!(offered, [nandi(INSTRUCTIONS_FILE)]);
    assert_eq!(
        update_files(layout, &next_build()).unwrap(),
        Updated::default()
    );
}

#[test]
fn a_gold_copy_from_0_1_0_tells_edited_from_unedited_by_its_hashes() {
    for (case, role, role_now) in [
        ("unedited role", INSTRUCTIONS_0_1_0, None),
        ("edited role", MY_ROLE, Some(MY_ROLE)),
    ] {
        let (_tmp, layout) = gold_copy_from_0_1_0(role);
        assert!(
            ShippedManifest::load(&layout).unwrap().is_none(),
            "no record"
        );

        let updated = update(&layout).unwrap();

        let today = |name: &str| {
            shipped_files(&layout)
                .into_iter()
                .find(|file| file.path == nandi(name))
                .unwrap()
                .content
        };
        assert_eq!(
            read(&layout, &nandi(AGENT_FILE)).as_deref(),
            Some(today(AGENT_FILE)),
            "{case}: the unedited agent.md is today's"
        );
        let role_expected = role_now.unwrap_or(today(INSTRUCTIONS_FILE));
        assert_eq!(
            read(&layout, &nandi(INSTRUCTIONS_FILE)).as_deref(),
            Some(role_expected),
            "{case}"
        );
        // New since 0.1.0: the playbooks and the default triggers
        for file in shipped_files(&layout) {
            assert!(
                layout.root.join(&file.path).is_file(),
                "{case}: {}",
                file.path
            );
        }
        assert!(
            updated.added.contains(&nandi("skills/know-me.md")),
            "{case}"
        );
        let offered: Vec<String> = offers(&layout)
            .unwrap()
            .into_iter()
            .map(|offer| offer.path)
            .collect();
        let expected: Vec<String> = role_now
            .map(|_| nandi(INSTRUCTIONS_FILE))
            .into_iter()
            .collect();
        assert_eq!(offered, expected, "{case}");
        assert!(
            ShippedManifest::load(&layout).unwrap().is_some(),
            "{case}: recorded"
        );
    }
}

#[test]
fn a_malformed_record_changes_nothing() {
    let (_tmp, store, _git) = gold_copy();
    let layout = store.layout();
    write(layout, ".nandi/shipped.yaml", "files: [not, a, map]\n");
    let skill = read(layout, &nandi(AGENT_FILE));
    assert!(update_files(layout, &next_build()).is_err());
    assert_eq!(read(layout, &nandi(AGENT_FILE)), skill);
}

// --- Take update and Keep mine -----------------------------------------------

/// A gold copy whose role the person edited, with the next build's
/// version of it on offer
fn with_role_on_offer() -> (tempfile::TempDir, RepoStore, GitService) {
    let (tmp, store, git) = gold_copy();
    write(store.layout(), &nandi(INSTRUCTIONS_FILE), MY_ROLE);
    (tmp, store, git)
}

#[test]
fn an_offer_shows_the_persons_version_this_builds_and_the_diff() {
    let (_tmp, store, _git) = with_role_on_offer();
    let offers = offers_of(store.layout(), &next_build()).unwrap();
    let offer = offers
        .iter()
        .find(|offer| offer.path == nandi(INSTRUCTIONS_FILE))
        .unwrap();
    assert_eq!(offer.mine, MY_ROLE);
    assert_eq!(offer.shipped, next_instructions());
    assert!(
        offer
            .diff
            .contains("-- **Name and title:** Ada, finance lead"),
        "{}",
        offer.diff
    );
    assert!(
        offer.diff.contains("+- **Working hours:**"),
        "{}",
        offer.diff
    );
}

#[test]
fn take_update_writes_this_builds_version_and_commits_it() {
    let (_tmp, store, git) = with_role_on_offer();
    let layout = store.layout();
    let path = nandi(INSTRUCTIONS_FILE);

    take_of(&store, &git, &next_build(), &path).unwrap();

    assert_eq!(read(layout, &path).as_deref(), Some(next_instructions()));
    assert_eq!(
        committed(layout, &path).as_deref(),
        Some(next_instructions())
    );
    assert_eq!(
        head_message(layout),
        format!("Take Nandi's update to {path}")
    );
    assert_eq!(
        manifest(layout).files.get(&path),
        Some(&hash(next_instructions()))
    );
    assert!(
        store
            .skill(DEFAULT_AGENT)
            .unwrap()
            .instructions
            .contains("Working hours"),
        "projected"
    );
    // The person's version stays in the history
    let (repo, oid) = head(layout);
    let before = repo.find_commit(oid).unwrap().parent(0).unwrap();
    assert_eq!(before.message(), Some(keep_before_take(&path).as_str()));
    assert!(offers_of(layout, &next_build()).unwrap().is_empty());
}

#[test]
fn keep_mine_is_remembered_until_a_newer_version_ships() {
    let (_tmp, store, git) = with_role_on_offer();
    let layout = store.layout();
    let path = nandi(INSTRUCTIONS_FILE);

    keep_mine_of(layout, &git, &next_build(), &path).unwrap();

    assert_eq!(read(layout, &path).as_deref(), Some(MY_ROLE));
    assert_eq!(
        manifest(layout).kept.get(&path),
        Some(&hash(next_instructions()))
    );
    assert_eq!(
        head_message(layout),
        format!("Keep my {path} over Nandi's update")
    );
    assert!(
        offers_of(layout, &next_build()).unwrap().is_empty(),
        "not again"
    );

    let newer = [agent_file(path.clone(), "# My role\n\n- **Pronouns:**\n")];
    assert_eq!(
        offers_of(layout, &newer).unwrap().len(),
        1,
        "a newer one is"
    );
}

#[test]
fn only_an_update_on_offer_can_be_taken_or_kept() {
    let (_tmp, store, git) = gold_copy();
    let layout = store.layout();
    let unedited = nandi(AGENT_FILE);
    let error = take_of(&store, &git, &next_build(), &unedited).unwrap_err();
    assert!(error.contains("no update"), "{error}");
    let error = keep_mine_of(layout, &git, &next_build(), ".nandi/settings.yaml").unwrap_err();
    assert!(
        error.contains("isn't one of Nandi's shipped files"),
        "{error}"
    );
}

// --- Reset to defaults -----------------------------------------------------

#[test]
fn a_reset_writes_the_shipped_files_records_them_and_forgets_keep_mine() {
    let (_tmp, store, git) = with_role_on_offer();
    let layout = store.layout();
    let path = nandi(INSTRUCTIONS_FILE);
    keep_mine_of(layout, &git, &next_build(), &path).unwrap();

    builtin_agents::reset_default(&store, &git).unwrap();

    let record = manifest(layout);
    assert!(record.kept.is_empty(), "Keep mine is forgotten");
    for file in shipped_files(layout) {
        assert_eq!(
            read(layout, &file.path).as_deref(),
            Some(file.content),
            "{} byte for byte",
            file.path
        );
        assert_eq!(
            record.files.get(&file.path),
            Some(&file.hash()),
            "{}",
            file.path
        );
    }
    assert_eq!(
        committed(layout, ".nandi/shipped.yaml"),
        read(layout, ".nandi/shipped.yaml"),
        "the record is in the reset's commit"
    );
    assert_eq!(
        update(layout).unwrap(),
        Updated::default(),
        "nothing to update"
    );
}
