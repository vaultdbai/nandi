use super::*;
use crate::services::builtin_agents;
use crate::services::repo_bootstrap::GITIGNORE;

/// The Chief of Staff's files as Nandi 0.1.0 seeded them (from the tag)
const SKILL_0_1_0: &str = include_str!("fixtures/v0_1_0/chief-of-staff/skill.md");
const INSTRUCTIONS_0_1_0: &str = include_str!("fixtures/v0_1_0/chief-of-staff/instructions.md");

const SETTINGS: &str = "\
default_model: deepseek:deepseek-flash
connector_grants:
- agent: chief-of-staff
  connector: outlook
  access: read
- agent: writer
  connector: drive
  access: read
";
const TRIGGER: &str = "\
id: weekly-report
name: Weekly report
source:
  type: cron
  expr: 0 9 * * MON
target:
  type: agent
  id: chief-of-staff
input_template: Ask chief-of-staff for the weekly report
";
const WRITER_SKILL: &str = "---\nname: Writer\ndescription: Drafts posts\n---\n\nYou write.\n";
const EDITED_ROLE: &str = "# My role\n\n- **Name and title:** Ada, finance lead\n";

/// A gold copy as Nandi 0.1.0 left it: committed, with the Chief of Staff
/// (its role edited when `edited`), a person's agent, a grant and a
/// trigger for the Chief of Staff, and a machine-local `work/` file
fn repo_from_0_1_0(edited: bool) -> (tempfile::TempDir, RepoLayout) {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    git2::Repository::init(&layout.root).unwrap();
    let instructions = if edited {
        EDITED_ROLE
    } else {
        INSTRUCTIONS_0_1_0
    };
    let files = [
        (".gitignore", GITIGNORE),
        (".nandi/settings.yaml", SETTINGS),
        (".nandi/agents/chief-of-staff/skill.md", SKILL_0_1_0),
        (".nandi/agents/chief-of-staff/instructions.md", instructions),
        (".nandi/agents/writer/skill.md", WRITER_SKILL),
        (".nandi/triggers/weekly-report.yaml", TRIGGER),
    ];
    for (file, text) in files {
        write(&layout, file, text);
    }
    let paths: Vec<String> = files.iter().map(|(file, _)| file.to_string()).collect();
    GitService::new(&layout.root)
        .commit_files(&paths, "Nandi 0.1.0", None)
        .unwrap();
    write(
        &layout,
        ".nandi/agents/chief-of-staff/work/scratch.txt",
        "local",
    );
    (tmp, layout)
}

fn write(layout: &RepoLayout, file: &str, text: &str) {
    let path = layout.root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(layout: &RepoLayout, file: &str) -> Option<String> {
    std::fs::read_to_string(layout.root.join(file)).ok()
}

/// A file in Nandi's folder, relative to the repo
fn nandi_file(name: &str) -> String {
    format!(".nandi/agents/{DEFAULT_AGENT}/{name}")
}

/// Edits Nandi's folder the way a person would
fn edit_nandi(layout: &RepoLayout) {
    write(layout, &nandi_file("instructions.md"), "# Mine\n");
}

/// A file at `HEAD~back`, if it is there
fn committed(layout: &RepoLayout, back: usize, file: &str) -> Option<String> {
    let repo = git2::Repository::open(&layout.root).unwrap();
    let mut commit = repo.head().unwrap().peel_to_commit().unwrap();
    for _ in 0..back {
        commit = commit.parent(0).unwrap();
    }
    let entry = commit.tree().unwrap().get_path(Path::new(file)).ok()?;
    let blob = repo.find_blob(entry.id()).unwrap();
    Some(String::from_utf8(blob.content().to_vec()).unwrap())
}

fn head_message(layout: &RepoLayout) -> String {
    let repo = git2::Repository::open(&layout.root).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    head.message().unwrap().to_string()
}

#[test]
fn the_0_1_0_fixture_is_what_0_1_0_shipped() {
    let fixture = [
        (SKILL_FILE, SKILL_0_1_0),
        (INSTRUCTIONS_FILE, INSTRUCTIONS_0_1_0),
    ]
    .map(|(file, text)| (file.to_string(), text_hash(text.as_bytes())));
    assert_eq!(BTreeMap::from(fixture), shipped_in_0_1_0());
}

#[test]
fn the_folder_kept_as_nandis_is_the_edited_one() {
    struct Case {
        name: &'static str,
        chief_of_staff_edited: bool,
        /// Nandi's folder before the upgrade: none, fresh, or edited
        nandi: Option<bool>,
        kept: Kept,
        role: &'static str,
    }
    let cases = [
        Case {
            name: "only the Chief of Staff, unedited",
            chief_of_staff_edited: false,
            nandi: None,
            kept: Kept::Moved,
            role: INSTRUCTIONS_0_1_0,
        },
        Case {
            name: "only the Chief of Staff, edited",
            chief_of_staff_edited: true,
            nandi: None,
            kept: Kept::Moved,
            role: EDITED_ROLE,
        },
        Case {
            name: "an edited Chief of Staff and a fresh Nandi",
            chief_of_staff_edited: true,
            nandi: Some(false),
            kept: Kept::ChiefOfStaffOverFreshNandi,
            role: EDITED_ROLE,
        },
        Case {
            name: "an unedited Chief of Staff and an edited Nandi",
            chief_of_staff_edited: false,
            nandi: Some(true),
            kept: Kept::NandiOverUneditedChiefOfStaff,
            role: "# Mine\n",
        },
        Case {
            name: "both edited",
            chief_of_staff_edited: true,
            nandi: Some(true),
            kept: Kept::NandiOverEditedChiefOfStaff,
            role: "# Mine\n",
        },
    ];
    for case in cases {
        let (_tmp, layout) = repo_from_0_1_0(case.chief_of_staff_edited);
        if let Some(nandi_edited) = case.nandi {
            builtin_agents::seed(&layout).unwrap();
            if nandi_edited {
                edit_nandi(&layout);
            }
        }

        let upgraded = upgrade(&layout).unwrap().unwrap();
        assert_eq!(upgraded.kept, case.kept, "{}", case.name);
        assert_eq!(
            read(&layout, &nandi_file("instructions.md")).as_deref(),
            Some(case.role),
            "{}",
            case.name
        );
        assert!(!layout.agent_dir(OLD_ID).exists(), "{}", case.name);
        // Committed as it is on disk, with the reason in the message
        assert_eq!(
            committed(&layout, 0, &nandi_file("instructions.md")).as_deref(),
            Some(case.role),
            "{}",
            case.name
        );
        let message = head_message(&layout);
        assert!(message.starts_with(COMMIT_MESSAGE), "{message}");
        assert!(message.contains(&case.kept.explain()), "{message}");
    }
}

#[test]
fn the_upgrade_commits_once_and_history_keeps_the_old_files() {
    let (_tmp, layout) = repo_from_0_1_0(true);

    let upgraded = upgrade(&layout).unwrap().unwrap();
    assert!(upgraded.commit.is_some());
    assert_eq!(head_message(&layout).lines().next(), Some(COMMIT_MESSAGE));
    // The commit before has the Chief of Staff as 0.1.0 left it
    assert_eq!(
        committed(&layout, 1, ".nandi/agents/chief-of-staff/instructions.md").as_deref(),
        Some(EDITED_ROLE)
    );
    assert_eq!(
        committed(&layout, 0, ".nandi/agents/chief-of-staff/skill.md"),
        None
    );
    assert_eq!(
        committed(&layout, 0, &nandi_file("skill.md")).as_deref(),
        Some(SKILL_0_1_0)
    );
    // Machine-local files move with the folder but are never committed
    assert_eq!(
        read(&layout, &nandi_file("work/scratch.txt")).as_deref(),
        Some("local")
    );
    assert_eq!(committed(&layout, 0, &nandi_file("work/scratch.txt")), None);
    // Nothing left to do on the next start
    assert_eq!(upgrade(&layout).unwrap(), None);
}

#[test]
fn grants_and_triggers_name_nandi() {
    let (_tmp, layout) = repo_from_0_1_0(false);

    let upgraded = upgrade(&layout).unwrap().unwrap();
    assert_eq!(
        upgraded.renamed_in,
        [".nandi/settings.yaml", ".nandi/triggers/weekly-report.yaml"]
    );
    let settings = read(&layout, ".nandi/settings.yaml").unwrap();
    assert!(
        settings.contains(&format!("agent: {DEFAULT_AGENT}")),
        "{settings}"
    );
    assert!(settings.contains("agent: writer"), "{settings}");
    assert!(!settings.contains(OLD_ID), "{settings}");
    let trigger = read(&layout, ".nandi/triggers/weekly-report.yaml").unwrap();
    assert!(
        trigger.contains(&format!("id: {DEFAULT_AGENT}")),
        "{trigger}"
    );
    // Words in a sentence are the person's: only whole IDs change
    assert!(
        trigger.contains("Ask chief-of-staff for the weekly report"),
        "{trigger}"
    );
    // The person's own agent is untouched
    assert_eq!(
        read(&layout, ".nandi/agents/writer/skill.md").as_deref(),
        Some(WRITER_SKILL)
    );
    assert_eq!(
        committed(&layout, 0, ".nandi/settings.yaml"),
        Some(settings)
    );
    let message = head_message(&layout);
    assert!(
        message.contains(&format!(
            "{OLD_ID} is now {DEFAULT_AGENT} in: .nandi/settings.yaml"
        )),
        "{message}"
    );
}

#[test]
fn a_repo_without_the_chief_of_staff_has_nothing_to_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    crate::services::repo_bootstrap::bootstrap(&layout).unwrap();
    let before = committed(&layout, 0, ".nandi/settings.yaml");

    assert_eq!(upgrade(&layout).unwrap(), None);
    assert_eq!(committed(&layout, 0, ".nandi/settings.yaml"), before);
}

#[test]
fn rename_agent_changes_only_whole_ids() {
    let nandi = DEFAULT_AGENT;
    let cases = [
        (
            "agent: chief-of-staff
",
            Some(format!(
                "agent: {nandi}
"
            )),
        ),
        (
            "steps:
- agent: chief-of-staff
  input: hi
",
            Some(format!(
                "steps:
- agent: {nandi}
  input: hi
"
            )),
        ),
        (
            "delegates: [writer, chief-of-staff]
",
            Some(format!(
                "delegates:
- writer
- {nandi}
"
            )),
        ),
        (
            "note: ask chief-of-staff
",
            None,
        ),
        (
            "agent: writer
",
            None,
        ),
    ];
    for (text, expected) in cases {
        let renamed = super::super::formats::SETTINGS
            .edit(text, rename_agent)
            .unwrap();
        assert_eq!(renamed, expected, "{text}");
    }
}

#[test]
fn line_endings_dont_make_a_file_edited() {
    let crlf = SKILL_0_1_0.replace('\n', "\r\n");
    assert_eq!(
        text_hash(crlf.as_bytes()),
        text_hash(SKILL_0_1_0.as_bytes())
    );
}
