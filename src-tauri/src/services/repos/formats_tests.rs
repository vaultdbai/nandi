use super::*;
use crate::services::repo_bootstrap::{self, commit_format_upgrade};
use crate::services::repo_service::GitService;
use crate::services::repos::{RepoLayout, Repos, Repository};

/// The example upgrade: format 2 renamed `title` to `name`
fn rename_title_to_name(yaml: &mut Mapping) -> Result<(), String> {
    if let Some(title) = yaml.remove("title") {
        yaml.insert("name".into(), title);
    }
    Ok(())
}

/// A kind of file at format 2, with one upgrade step
const EXAMPLE: FileFormat =
    FileFormat::new("example", "examples", Syntax::Yaml, &[rename_title_to_name]);
const EXAMPLE_MD: FileFormat = FileFormat::new(
    "note",
    "notes",
    Syntax::Frontmatter,
    &[rename_title_to_name],
);

#[test]
fn reading_gives_the_domain_current_text_without_the_version() {
    let newer = Err(FormatError::Newer {
        found: 3,
        current: 2,
    });
    let cases: &[(&str, &FileFormat, &str, Result<&str, FormatError>)] = &[
        (
            "today's file, untouched",
            &WORKFLOW,
            "id: a\n",
            Ok("id: a\n"),
        ),
        (
            "format 1 upgrades in memory",
            &EXAMPLE,
            "title: A\n",
            Ok("name: A\n"),
        ),
        (
            "explicit format 1",
            &EXAMPLE,
            "format_version: 1\ntitle: A\n",
            Ok("name: A\n"),
        ),
        (
            "current, key removed",
            &EXAMPLE,
            "format_version: 2\nname: A\n",
            Ok("name: A\n"),
        ),
        (
            "from a newer Nandi",
            &EXAMPLE,
            "format_version: 3\nname: A\n",
            newer,
        ),
        (
            "frontmatter, body kept",
            &EXAMPLE_MD,
            "---\ntitle: A\n---\n# Body\ntitle: stays\n",
            Ok("---\nname: A\n---\n# Body\ntitle: stays\n"),
        ),
    ];
    for (case, format, text, expected) in cases {
        let read = format.read(text);
        assert_eq!(
            read.as_deref().map_err(Clone::clone),
            expected.clone(),
            "{case}"
        );
    }
}

#[test]
fn a_broken_version_is_reported() {
    for text in [
        "format_version: 0\n",
        "format_version: two\n",
        "format_version: -1\n",
    ] {
        let error = EXAMPLE.read(text).unwrap_err();
        assert!(
            matches!(error, FormatError::Invalid(_)),
            "{text}: {error:?}"
        );
        assert!(error.to_string().starts_with("format_version:"), "{error}");
    }
}

#[test]
fn saved_text_carries_the_current_version_once_there_is_a_second() {
    // Format 1 kinds write exactly what the domain wrote
    assert_eq!(TRIGGER.stamp("id: t\n"), "id: t\n");
    assert_eq!(EXAMPLE.stamp("name: A\n"), "format_version: 2\nname: A\n");
    assert_eq!(
        EXAMPLE_MD.stamp("---\nname: A\n---\nBody\n"),
        "---\nformat_version: 2\nname: A\n---\nBody\n"
    );
    // A person's text: no version means format 1, so it is upgraded first
    assert_eq!(
        EXAMPLE.canonical("title: A\n").unwrap(),
        "format_version: 2\nname: A\n"
    );
    let current = "format_version: 2\nname: A\n";
    assert!(matches!(
        EXAMPLE.canonical(current).unwrap(),
        Cow::Borrowed(_)
    ));
}

#[test]
fn the_commit_message_counts_each_kind() {
    let mut upgrade = FormatUpgrade::default();
    upgrade.add(&EXAMPLE, "a.yaml".into());
    assert_eq!(upgrade.commit_message(), "Upgrade 1 example to format 2");
    upgrade.add(&EXAMPLE, "b.yaml".into());
    upgrade.add(&EXAMPLE, "c.yaml".into());
    upgrade.add(&EXAMPLE_MD, "d.md".into());
    assert_eq!(
        upgrade.commit_message(),
        "Upgrade 3 examples to format 2, 1 note to format 2"
    );
}

/// A gold copy with its first commit, and a file in it
fn repo_with(file: &str, text: &str) -> (tempfile::TempDir, RepoLayout, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    repo_bootstrap::bootstrap(&layout).unwrap();
    let path = layout.root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    let git = GitService::new(&layout.root);
    git.commit_files(&[file.to_string()], "Add it", None)
        .unwrap();
    (tmp, layout, path)
}

#[test]
fn an_old_file_upgrades_and_commits_once() {
    let file = ".nandi/examples/a.yaml";
    let (_tmp, layout, path) = repo_with(file, "title: A\n");
    let repos = Repos::new(layout.clone());
    let files = [(&EXAMPLE, path.clone())];

    let upgrade = repos.upgrade_files(&files);
    assert_eq!(upgrade.paths, std::slice::from_ref(&path));
    let commit = commit_format_upgrade(&layout, upgrade).unwrap();
    assert!(commit.is_some());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "format_version: 2\nname: A\n"
    );
    let repo = git2::Repository::open(&layout.root).unwrap();
    let upgraded = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(upgraded.message(), Some("Upgrade 1 example to format 2"));
    // The old text stays in history
    assert_eq!(
        blob_at(&repo, &upgraded.parent(0).unwrap(), file),
        "title: A\n"
    );
    assert_eq!(
        blob_at(&repo, &upgraded, file),
        "format_version: 2\nname: A\n"
    );
    // The working tree is clean, and a second start does nothing
    assert!(repo.statuses(None).unwrap().is_empty());
    let again = repos.upgrade_files(&files);
    assert!(again.is_empty());
    assert_eq!(commit_format_upgrade(&layout, again).unwrap(), None);
    assert_eq!(
        repo.head().unwrap().peel_to_commit().unwrap().id(),
        upgraded.id()
    );
}

#[test]
fn current_and_newer_files_are_never_rewritten() {
    let cases = [
        ("current", "format_version: 2\nname: A\n"),
        ("from a newer Nandi", "format_version: 3\nname: A\n"),
        ("broken YAML", "name: [A\n"),
    ];
    for (case, text) in cases {
        let file = ".nandi/examples/a.yaml";
        let (_tmp, layout, path) = repo_with(file, text);
        let repo = git2::Repository::open(&layout.root).unwrap();
        let before = repo.head().unwrap().peel_to_commit().unwrap().id();

        let upgrade = Repos::new(layout.clone()).upgrade_files(&[(&EXAMPLE, path.clone())]);
        assert!(upgrade.is_empty(), "{case}");
        assert_eq!(commit_format_upgrade(&layout, upgrade).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{case}");
        assert_eq!(repo.head().unwrap().peel_to_commit().unwrap().id(), before);
    }
}

#[test]
fn a_definition_from_a_newer_nandi_is_refused_with_a_calm_message() {
    let (_tmp, layout, _) = repo_with("README.md", "Mine\n");
    let path = layout.trigger_path("t");
    std::fs::write(&path, "format_version: 9\n").unwrap();
    let repos = Repos::new(layout.clone());
    let loaded = repos.triggers.load_all();
    assert!(loaded.entities.iter().all(|t| t.id().as_str() != "t"));
    let problem = loaded
        .problems
        .iter()
        .find(|p| p.id.as_deref() == Some("t"))
        .unwrap();
    let message = problem.error.to_string();
    assert!(
        message.contains("written by a newer version of Nandi (format 9"),
        "{message}"
    );
    // The startup pass leaves it as it is
    assert!(repos.upgrade_formats().is_empty());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "format_version: 9\n"
    );
}

#[test]
fn today_every_definition_kind_is_at_format_1_and_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
    repo_bootstrap::bootstrap(&layout).unwrap();
    let repos = Repos::new(layout);
    for format in [&AGENT, &WORKFLOW, &TRIGGER, &SETTINGS, &CONNECTORS] {
        assert_eq!(format.current(), 1, "{}", format.noun);
    }
    let listed: Vec<&str> = repos
        .definition_files()
        .iter()
        .map(|(format, _)| format.noun)
        .collect();
    // A new repo has Nandi's agent, its default triggers and the settings
    for noun in ["agent", "trigger", "settings file"] {
        assert!(listed.contains(&noun), "{noun} in {listed:?}");
    }
    assert!(repos.upgrade_formats().is_empty());
}

fn blob_at(repo: &git2::Repository, commit: &git2::Commit, file: &str) -> String {
    let entry = commit.tree().unwrap().get_path(Path::new(file)).unwrap();
    let blob = repo.find_blob(entry.id()).unwrap();
    String::from_utf8(blob.content().to_vec()).unwrap()
}

#[test]
fn an_edit_keeps_the_file_at_its_format() {
    let rename_a = |yaml: &mut Mapping| match yaml.get_mut("agent") {
        Some(value) if value.as_str() == Some("a") => {
            *value = "b".into();
            true
        }
        _ => false,
    };
    let cases = [
        ("agent: a\n", Some("agent: b\n")),
        (
            "format_version: 1\nagent: a\n",
            Some("format_version: 1\nagent: b\n"),
        ),
        ("agent: c\n", None),
    ];
    for (text, expected) in cases {
        let edited = SETTINGS.edit(text, rename_a).unwrap();
        assert_eq!(edited.as_deref(), expected, "{text}");
    }
    let newer = SETTINGS.edit("format_version: 9\nagent: a\n", rename_a);
    assert_eq!(
        newer,
        Err(FormatError::Newer {
            found: 9,
            current: 1
        })
    );
}
