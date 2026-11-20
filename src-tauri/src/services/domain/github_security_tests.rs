//! Tests for the GitHub security check's rules, over GitHub's JSON

use chrono::NaiveDate;
use serde_json::{json, Value};

use super::*;

const SCOPES: &[&str] = &["repo", "read:org", "read:user"];

fn snapshot(user: Value) -> Snapshot {
    Snapshot {
        scopes: Some(SCOPES.iter().map(|s| (*s).to_string()).collect()),
        user: Fetched::Got(user),
        backup: None,
        orgs: Fetched::Got(Vec::new()),
    }
}

fn outcome(run: &CheckRun, source_ref: &str) -> Outcome {
    run.checks
        .iter()
        .find(|check| check.source_ref == source_ref)
        .unwrap_or_else(|| panic!("no check {source_ref} in {run:?}"))
        .outcome
        .clone()
}

fn fail_text(run: &CheckRun, source_ref: &str) -> String {
    match outcome(run, source_ref) {
        Outcome::Fail(text) => text,
        other => panic!("{source_ref} is {other:?}"),
    }
}

fn repo(name: &str, scanning: &str, protected: bool, rules: Value) -> RepoSnapshot {
    RepoSnapshot {
        info: json!({
            "name": name,
            "default_branch": "main",
            "security_and_analysis": {
                "secret_scanning": { "status": scanning },
                "secret_scanning_push_protection": { "status": "enabled" },
            },
        }),
        branch: Fetched::Got(json!({ "name": "main", "protected": protected })),
        rules: Fetched::Got(rules),
    }
}

fn org(login: &str, info: Fetched, repos: Vec<RepoSnapshot>) -> OrgSnapshot {
    OrgSnapshot {
        login: login.to_string(),
        info,
        repos: Fetched::Got(repos),
        unread_repos: 0,
    }
}

fn existing(task_id: &str, source_ref: &str, open: bool, note: Option<&str>) -> ExistingCheck {
    ExistingCheck {
        task_id: task_id.to_string(),
        source_ref: source_ref.to_string(),
        open,
        note: note.map(str::to_string),
    }
}

#[test]
fn account_two_factor_off_is_a_finding_and_on_passes() {
    let off = evaluate(&snapshot(json!({ "two_factor_authentication": false })));
    let text = fail_text(&off, "github:user:2fa");
    assert!(text.starts_with("Turn on two-factor sign-in"), "{text}");
    assert!(
        text.contains("https://github.com/settings/security"),
        "{text}"
    );
    assert!(text.chars().count() <= MAX_TEXT_CHARS);

    let on = evaluate(&snapshot(json!({ "two_factor_authentication": true })));
    assert_eq!(outcome(&on, "github:user:2fa"), Outcome::Pass);
    assert_eq!(outcome(&on, "github:scope:read"), Outcome::Pass);
    assert_eq!(on.failed(), 0);
}

#[test]
fn a_hidden_field_is_never_guessed() {
    // GitHub left the private field out: unknown, not off
    let run = evaluate(&snapshot(json!({ "login": "alice" })));
    assert_eq!(outcome(&run, "github:user:2fa"), Outcome::Unknown);
    let mut unread = snapshot(json!({}));
    unread.user = Fetched::Unread;
    assert_eq!(
        outcome(&evaluate(&unread), "github:user:2fa"),
        Outcome::Unknown
    );
}

#[test]
fn a_public_backup_repo_is_a_finding_and_a_private_one_passes() {
    let mut public = snapshot(json!({ "two_factor_authentication": true }));
    public.backup = Some(BackupSnapshot {
        owner: "Alice".into(),
        repo: "Nandi-Backup".into(),
        info: Fetched::Got(json!({ "private": false, "visibility": "public" })),
    });
    let run = evaluate(&public);
    let text = fail_text(&run, "github:repo:alice/nandi-backup:public-backup");
    assert!(text.contains("Alice/Nandi-Backup"), "{text}");
    assert!(
        text.contains("https://github.com/Alice/Nandi-Backup/settings"),
        "{text}"
    );

    let mut private = public.clone();
    private.backup.as_mut().unwrap().info = Fetched::Got(json!({ "private": true }));
    let run = evaluate(&private);
    assert_eq!(
        outcome(&run, "github:repo:alice/nandi-backup:public-backup"),
        Outcome::Pass
    );
    let mut hidden = public;
    hidden.backup.as_mut().unwrap().info = Fetched::Hidden;
    assert_eq!(
        outcome(
            &evaluate(&hidden),
            "github:repo:alice/nandi-backup:public-backup"
        ),
        Outcome::Unknown
    );
}

#[test]
fn backup_remotes_on_github_are_recognised() {
    let pair = |o: &str, r: &str| Some((o.to_string(), r.to_string()));
    assert_eq!(
        github_repo_of("https://github.com/alice/backup.git"),
        pair("alice", "backup")
    );
    assert_eq!(
        github_repo_of(" https://www.github.com/alice/backup/ "),
        pair("alice", "backup")
    );
    assert_eq!(
        github_repo_of("git@github.com:alice/backup.git"),
        pair("alice", "backup")
    );
    assert_eq!(
        github_repo_of("ssh://git@github.com/alice/backup"),
        pair("alice", "backup")
    );
    assert_eq!(github_repo_of("https://gitlab.com/alice/backup"), None);
    assert_eq!(github_repo_of("https://github.com.evil.example/a/b"), None);
    assert_eq!(github_repo_of("http://github.com/alice/backup"), None);
    assert_eq!(github_repo_of("https://github.com/alice"), None);
    assert_eq!(github_repo_of("https://github.com/alice/b/tree/main"), None);
    assert_eq!(github_repo_of("/tmp/backup"), None);
}

#[test]
fn only_plain_names_go_in_an_api_path() {
    assert!(is_github_name("acme-corp"));
    assert!(is_github_name("my.repo_1"));
    for bad in ["", ".", "..", "a/b", "a?b", "a@evil.com", "a b", "a%2F"] {
        assert!(!is_github_name(bad), "{bad}");
    }
    assert!(is_branch_name("main"));
    assert!(is_branch_name("release/v1.2"));
    for bad in ["", "/main", "a/../b", "main?x", "a#b", "ma in"] {
        assert!(!is_branch_name(bad), "{bad}");
    }
}

#[test]
fn an_org_that_does_not_require_two_factor_is_a_finding() {
    let mut s = snapshot(json!({ "two_factor_authentication": true }));
    s.orgs = Fetched::Got(vec![
        org(
            "Acme",
            Fetched::Got(json!({ "two_factor_requirement_enabled": false })),
            Vec::new(),
        ),
        org(
            "Safe",
            Fetched::Got(json!({ "two_factor_requirement_enabled": true })),
            Vec::new(),
        ),
    ]);
    let run = evaluate(&s);
    let text = fail_text(&run, "github:org:acme:2fa");
    assert!(text.contains("organization Acme"), "{text}");
    assert!(
        text.contains("https://github.com/organizations/Acme/settings/authentication_security"),
        "{text}"
    );
    assert_eq!(outcome(&run, "github:org:safe:2fa"), Outcome::Pass);
    assert_eq!(outcome(&run, "github:org:safe:unreadable"), Outcome::Pass);
    assert_eq!(
        outcome(&run, "github:org:safe:secret-scanning"),
        Outcome::Pass
    );
    assert_eq!(run.failed(), 1);
}

#[test]
fn public_repos_with_secret_scanning_off_are_one_finding_with_names() {
    let mut s = snapshot(json!({ "two_factor_authentication": true }));
    let rules = json!([{ "type": "non_fast_forward" }, { "type": "deletion" }]);
    let repos = ["web", "api", "docs", "cli"]
        .iter()
        .map(|name| repo(name, "disabled", false, rules.clone()))
        .chain([repo("ok", "enabled", false, rules.clone())])
        .collect();
    let mut acme = org(
        "acme",
        Fetched::Got(json!({ "two_factor_requirement_enabled": true })),
        repos,
    );
    acme.unread_repos = 2;
    s.orgs = Fetched::Got(vec![acme]);
    let run = evaluate(&s);
    let text = fail_text(&run, "github:org:acme:secret-scanning");
    // 4 off, 2 past the cap: three named and 3 more
    assert!(text.contains("4 public repositories in acme"), "{text}");
    assert!(text.contains("(web, api, docs and 3 more)"), "{text}");
    assert!(
        text.contains("https://github.com/organizations/acme/settings/security_analysis"),
        "{text}"
    );
    assert_eq!(
        outcome(&run, "github:org:acme:branch-protection"),
        Outcome::Pass
    );
}

#[test]
fn a_default_branch_open_to_force_push_is_a_finding() {
    let mut s = snapshot(json!({ "two_factor_authentication": true }));
    s.orgs = Fetched::Got(vec![org(
        "acme",
        Fetched::Got(json!({ "two_factor_requirement_enabled": true, "public_repos": 3 })),
        vec![
            // Only one of the two rules: still open to deletion
            repo(
                "web",
                "enabled",
                false,
                json!([{ "type": "non_fast_forward" }]),
            ),
            // Classic protection
            repo("api", "enabled", true, json!([])),
            repo("docs", "enabled", false, json!([])),
        ],
    )]);
    let run = evaluate(&s);
    let text = fail_text(&run, "github:org:acme:branch-protection");
    assert!(
        text.contains("2 public repositories in acme (web and docs)"),
        "{text}"
    );
    assert!(
        text.contains("https://github.com/organizations/acme/settings/rules"),
        "{text}"
    );
}

#[test]
fn repos_github_did_not_show_are_unknown_not_findings() {
    let mut s = snapshot(json!({ "two_factor_authentication": true }));
    let mut hidden = repo("web", "enabled", false, json!([]));
    hidden.info = json!({ "name": "web" });
    hidden.branch = Fetched::Unread;
    hidden.rules = Fetched::Unread;
    s.orgs = Fetched::Got(vec![org(
        "acme",
        Fetched::Got(json!({ "two_factor_requirement_enabled": true, "public_repos": 1 })),
        vec![hidden],
    )]);
    let run = evaluate(&s);
    assert_eq!(
        outcome(&run, "github:org:acme:secret-scanning"),
        Outcome::Unknown
    );
    assert_eq!(
        outcome(&run, "github:org:acme:branch-protection"),
        Outcome::Unknown
    );
}

#[test]
fn hidden_org_settings_ask_to_sign_in_again_or_approve_the_app() {
    // An old sign-in, `repo` only: memberships are hidden
    let mut old = snapshot(json!({ "two_factor_authentication": true }));
    old.scopes = Some(vec!["repo".into()]);
    old.orgs = Fetched::Hidden;
    let run = evaluate(&old);
    let text = fail_text(&run, "github:scope:read");
    assert!(text.starts_with("Couldn't check your GitHub organization settings"));
    assert!(text.contains("sign in to GitHub again"), "{text}");
    assert!(!run.settled.contains(&"github:org:".to_string()));

    // The right scopes, but the org hides its settings: it limits apps
    let mut limited = snapshot(json!({ "two_factor_authentication": true }));
    limited.orgs = Fetched::Got(vec![org(
        "acme",
        Fetched::Got(json!({ "login": "acme" })),
        Vec::new(),
    )]);
    let run = evaluate(&limited);
    assert_eq!(outcome(&run, "github:scope:read"), Outcome::Pass);
    assert_eq!(outcome(&run, "github:org:acme:2fa"), Outcome::Unknown);
    let text = fail_text(&run, "github:org:acme:unreadable");
    assert!(text.contains("oauth_application_policy"), "{text}");

    // GitHub unreachable: nothing is known, nothing is a finding
    let mut offline = snapshot(json!({}));
    offline.user = Fetched::Unread;
    offline.orgs = Fetched::Unread;
    offline.scopes = None;
    let run = evaluate(&offline);
    assert_eq!(run.failed(), 0);
    assert_eq!(outcome(&run, "github:scope:read"), Outcome::Unknown);
}

#[test]
fn every_finding_fits_in_a_todo() {
    let long = "r".repeat(100);
    let mut s = snapshot(json!({ "two_factor_authentication": false }));
    let repos = (0..5)
        .map(|i| repo(&format!("{long}{i}"), "disabled", false, json!([])))
        .collect();
    s.orgs = Fetched::Got(vec![org(
        &"o".repeat(39),
        Fetched::Got(json!({ "two_factor_requirement_enabled": false })),
        repos,
    )]);
    let run = evaluate(&s);
    for check in &run.checks {
        if let Outcome::Fail(text) = &check.outcome {
            assert!(text.chars().count() <= MAX_TEXT_CHARS, "{text}");
        }
    }
}

#[test]
fn a_finding_opens_one_todo_and_a_recheck_adds_none() {
    let run = evaluate(&snapshot(json!({ "two_factor_authentication": false })));
    let steps = plan(&run, &[]);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(matches!(&steps[0], Step::Add { source_ref, .. } if source_ref == "github:user:2fa"));

    let open = [existing("todo-1", "github:user:2fa", true, None)];
    assert_eq!(plan(&run, &open), Vec::new(), "still there: unchanged");
    // The user closed it by hand: it stays closed
    let dismissed = [existing("todo-1", "github:user:2fa", false, Some("later"))];
    assert_eq!(plan(&run, &dismissed), Vec::new());
}

#[test]
fn a_fixed_finding_is_closed_and_opens_again_if_it_comes_back() {
    let fixed = evaluate(&snapshot(json!({ "two_factor_authentication": true })));
    let open = [existing("todo-1", "github:user:2fa", true, None)];
    assert_eq!(
        plan(&fixed, &open),
        vec![Step::Close {
            task_id: "todo-1".into()
        }]
    );
    let note = fixed_note(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
    assert_eq!(note, "Fixed: checked again on 2026-11-20");

    let back = evaluate(&snapshot(json!({ "two_factor_authentication": false })));
    let closed = [existing("todo-1", "github:user:2fa", false, Some(&note))];
    assert_eq!(
        plan(&back, &closed),
        vec![Step::Reopen {
            task_id: "todo-1".into()
        }]
    );
    assert_eq!(plan(&fixed, &closed), Vec::new());
}

#[test]
fn unknown_checks_leave_todos_and_gone_orgs_close_theirs() {
    let mut offline = snapshot(json!({}));
    offline.user = Fetched::Unread;
    offline.orgs = Fetched::Unread;
    let open = [
        existing("t1", "github:user:2fa", true, None),
        existing("t2", "github:org:acme:2fa", true, None),
    ];
    assert_eq!(plan(&evaluate(&offline), &open), Vec::new());

    // Orgs listed, acme no longer among them; the Backup remote changed
    let mut listed = snapshot(json!({}));
    listed.orgs = Fetched::Got(Vec::new());
    let open = [
        existing("t2", "github:org:acme:2fa", true, None),
        existing("t3", "github:repo:alice/old:public-backup", true, None),
    ];
    assert_eq!(
        plan(&evaluate(&listed), &open),
        vec![
            Step::Close {
                task_id: "t2".into()
            },
            Step::Close {
                task_id: "t3".into()
            },
        ]
    );
}
