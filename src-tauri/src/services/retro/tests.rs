//! The retro over a real gold copy (a temporary folder with git) and an
//! in-memory `state.db`: proposing, the limits at apply time, apply and
//! undo, and the check from run records.

use std::sync::Arc;

use serde_json::json;

use super::check::RetroVerdict;
use super::proposals::ProposalStatus;
use super::*;
use crate::db::Db;
use crate::services::builtin_agents;
use crate::services::repo_service::GitService;
use crate::services::repo_store::RepoStore;

const WRITER: &str = "---
name: Writer
description: Drafts posts
allowed-tools: [read_file, write_file]
requires_approval: [write_file]
network:
  allow:
    - host: www.linkedin.com
---

Draft posts for the user.
";

const NOW: i64 = 1_800_000_000_000;

struct Fixture {
    retro: Retro,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let db = Db::in_memory();
        let store = RepoStore::for_tests(db.clone());
        let layout = store.layout().clone();
        builtin_agents::seed(&layout).unwrap();
        builtin_agents::seed_triggers(&layout).unwrap();
        let writer = layout.agent_dir("writer");
        std::fs::create_dir_all(&writer).unwrap();
        std::fs::write(writer.join("skill.md"), WRITER).unwrap();
        let pack = layout.vendored_agent_dir("acme", "poster");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("skill.md"), WRITER.replace("Writer", "Poster")).unwrap();
        let repo = git2::Repository::init(&layout.root).unwrap();
        crate::services::repo_bootstrap::commit_all(&repo, "Setup", Some("tester")).unwrap();
        store.load().unwrap();
        let conn = db.conn().unwrap();
        for run in ["r1", "r2"] {
            crate::db::tests::start_run(&conn, "writer", run);
        }
        conn.execute("UPDATE runs SET started_at = ?1", [NOW - DAY_MS])
            .unwrap();
        drop(conn);
        Self {
            root: layout.root.clone(),
            retro: Retro {
                store,
                git: Arc::new(GitService::new(&layout.root)),
                db,
            },
        }
    }

    fn propose(&self, agent: &str, target: &str, content: &str) -> Result<Proposal, String> {
        propose(
            &self.retro,
            "retro-run",
            ProposeInput {
                agent: agent.to_string(),
                target: target.to_string(),
                content: content.to_string(),
                reason: "You shortened 4 of 5 drafts".to_string(),
                runs: vec!["r1".to_string(), "r2".to_string()],
            },
            NOW,
        )
    }

    fn skill(&self) -> String {
        std::fs::read_to_string(self.root.join(".nandi/agents/writer/skill.md")).unwrap()
    }

    fn head(&self) -> (String, String) {
        let repo = git2::Repository::open(&self.root).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        (
            head.tree_id().to_string(),
            head.message().unwrap_or_default().to_string(),
        )
    }

    /// Writes a proposal straight into `state.db`, as if made before the
    /// limits: so apply's own check is what's tested
    fn sneak(&self, after: &str) -> String {
        let proposal = Proposal {
            id: "sneaky".to_string(),
            run_id: "retro-run".to_string(),
            agent_id: "writer".to_string(),
            target: "skill".to_string(),
            path: ".nandi/agents/writer/skill.md".to_string(),
            reason: "Wider".to_string(),
            runs: vec!["r1".to_string()],
            before: Some(WRITER.to_string()),
            after: after.to_string(),
            diff: String::new(),
            status: ProposalStatus::Pending,
            created_at: NOW,
            decided_at: None,
            commit_id: None,
            undo_commit_id: None,
            verdict: None,
            rates: None,
        };
        proposals::insert(&self.retro.db.conn().unwrap(), &proposal).unwrap();
        proposal.id
    }
}

const SHORTER: &str = "Draft posts for the user.\nKeep them under 80 words; no hashtags.\n";

#[test]
fn a_proposal_is_recorded_with_its_diff_and_changes_nothing() {
    let fx = Fixture::new();
    let after = WRITER.replace("Draft posts for the user.\n", SHORTER);
    let proposal = fx.propose("writer", "skill", &after).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Pending);
    assert_eq!(proposal.path, ".nandi/agents/writer/skill.md");
    assert!(
        proposal.diff.contains("+Keep them under 80 words"),
        "{}",
        proposal.diff
    );
    assert_eq!(proposal.runs, ["r1", "r2"]);
    assert_eq!(fx.skill(), WRITER, "nothing written before a yes");

    // The Inbox item: what changes, why, the runs, and when it stops waiting
    let items = items(&fx.retro, NOW).unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(
        (item.agent_name.as_str(), item.target.as_str()),
        ("Writer", "skill")
    );
    assert_eq!(item.reason, "You shortened 4 of 5 drafts");
    assert_eq!(item.runs, ["r1", "r2"]);
    assert!(item.diff.contains("+Keep them under 80 words"));
    assert_eq!(item.expires_at, Some(crate::db::ts(NOW + WAITS_MS)));
    assert!(!item.needs_note);
    let wire = serde_json::to_value(item).unwrap();
    for key in [
        "agentId",
        "agentName",
        "diff",
        "runs",
        "status",
        "expiresAt",
        "needsNote",
    ] {
        assert!(wire.get(key).is_some(), "{key} in {wire}");
    }
    assert_eq!(wire["status"], "pending");
    // Gone from the Inbox after a week
    assert!(super::items(&fx.retro, NOW + WAITS_MS + 1)
        .unwrap()
        .is_empty());
}

#[test]
fn a_proposal_must_cite_real_runs_and_keep_to_three_a_week() {
    let fx = Fixture::new();
    let mut input = ProposeInput {
        agent: "writer".into(),
        target: "playbook:posts".into(),
        content: "---\nname: posts\ndescription: Posts\nwhen: A post\n---\n\n1. Short.\n".into(),
        reason: "Shorter posts".into(),
        runs: vec!["nope".into()],
    };
    let refused = propose(&fx.retro, "retro-run", input.clone(), NOW).unwrap_err();
    assert!(refused.contains("no run 'nope'"), "{refused}");
    input.runs = vec!["r1".into()];
    input.reason = String::new();
    assert!(propose(&fx.retro, "retro-run", input.clone(), NOW).is_err());
    input.reason = "Shorter posts".into();
    for name in ["posts", "replies", "threads"] {
        input.target = format!("playbook:{name}");
        input.content = input
            .content
            .replace("name: posts", &format!("name: {name}"));
        propose(&fx.retro, "retro-run", input.clone(), NOW).unwrap();
        input.content = input
            .content
            .replace(&format!("name: {name}"), "name: posts");
    }
    input.target = "playbook:more".into();
    input.content = input.content.replace("name: posts", "name: more");
    let fourth = propose(&fx.retro, "retro-run", input, NOW).unwrap_err();
    assert!(fourth.contains("3 proposals this week"), "{fourth}");
}

#[test]
fn a_proposal_that_adds_a_tool_a_site_or_a_secret_is_refused() {
    let fx = Fixture::new();
    let wider = [
        WRITER.replace("[read_file, write_file]", "[read_file, write_file, shell]"),
        WRITER.replace("    - host: www.linkedin.com\n", "    - host: www.linkedin.com\n    - host: evil.example\n"),
        WRITER.replace(
            "---\n\nDraft",
            "pilots:\n  - id: browser\n    secrets: {SITE_PASSWORD: nandi.pilot.writer.site}\n---\n\nDraft",
        ),
        WRITER.replace("requires_approval: [write_file]", "requires_approval: []"),
    ];
    for after in &wider {
        let why = fx.propose("writer", "skill", after).unwrap_err();
        assert!(why.starts_with("Refused:"), "{why}");
    }
    // Made anyway (say, before a rule existed): refused again at apply
    for after in &wider {
        let id = fx.sneak(after);
        let why = apply(&fx.retro, &id, NOW).unwrap_err();
        assert!(why.starts_with("Refused:"), "{why}");
        assert_eq!(fx.skill(), WRITER, "nothing written");
        proposals::decide(
            &fx.retro.db.conn().unwrap(),
            &id,
            ProposalStatus::Pending,
            ProposalStatus::Refused,
            None,
            NOW,
        )
        .unwrap();
        fx.retro
            .db
            .conn()
            .unwrap()
            .execute("DELETE FROM retro_proposals WHERE id = ?1", [&id])
            .unwrap();
    }
    // Narrowing is fine
    let narrower = WRITER.replace("[read_file, write_file]", "[read_file]");
    fx.propose("writer", "skill", &narrower).unwrap();
}

#[test]
fn a_signed_pack_agent_is_never_changed() {
    let fx = Fixture::new();
    let why = fx
        .propose("poster", "skill", &WRITER.replace("Writer", "Poster!"))
        .unwrap_err();
    assert!(
        why.contains("signed pack") && why.contains("local copy"),
        "{why}"
    );
    let why = fx
        .propose(
            "poster",
            "playbook:posts",
            "---\nname: posts\ndescription: P\nwhen: W\n---\n\n1.\n",
        )
        .unwrap_err();
    assert!(why.contains("signed pack"), "{why}");
    let read = tool::call(
        &fx.retro,
        "retro-run",
        tool::READ,
        &json!({ "agent": "poster", "target": "skill" }),
        NOW,
    );
    assert!(read.is_error && read.result.contains("signed pack"));
}

#[test]
fn apply_then_undo_leaves_the_gold_copy_as_it_was() {
    let fx = Fixture::new();
    let (tree_before, _) = fx.head();
    let after = WRITER.replace("Draft posts for the user.\n", SHORTER);
    let id = fx.propose("writer", "skill", &after).unwrap().id;

    let applied = apply(&fx.retro, &id, NOW + 1).unwrap();
    assert_eq!(applied.status, ProposalStatus::Applied);
    assert_eq!(fx.skill(), after);
    let (tree_applied, message) = fx.head();
    assert_ne!(tree_applied, tree_before);
    assert!(
        message.starts_with("Weekly retro: You shortened 4 of 5 drafts"),
        "{message}"
    );
    assert!(
        message.contains("Runs it learned from: r1, r2"),
        "{message}"
    );
    assert_eq!(applied.commit_id.as_deref().map(str::len), Some(40));
    assert!(
        fx.retro
            .store
            .agent("writer")
            .unwrap()
            .skill_body()
            .contains("80 words"),
        "the agent runs with it"
    );
    // Applied: still in the Inbox this week, with its Undo
    assert_eq!(
        items(&fx.retro, NOW + 2).unwrap()[0].status,
        ProposalStatus::Applied
    );
    assert!(apply(&fx.retro, &id, NOW + 2).is_err(), "only once");

    let undone = undo(&fx.retro, &id, NOW + 3).unwrap();
    assert_eq!(undone.status, ProposalStatus::Undone);
    assert_eq!(fx.skill(), WRITER);
    let (tree_undone, message) = fx.head();
    assert_eq!(tree_undone, tree_before, "the gold copy is as it was");
    assert!(
        message.starts_with("Undo the weekly retro's change"),
        "{message}"
    );
    assert!(!fx
        .retro
        .store
        .agent("writer")
        .unwrap()
        .skill_body()
        .contains("80 words"));
    assert!(items(&fx.retro, NOW + 4).unwrap().is_empty());
}

#[test]
fn a_new_playbook_goes_on_apply_and_away_on_undo() {
    let fx = Fixture::new();
    let (tree_before, _) = fx.head();
    let playbook = "---\nname: linkedin-posts\ndescription: Write LinkedIn posts\nwhen: A post is due\n---\n\n1. Under 80 words.\n";
    let id = fx
        .propose("writer", "playbook:linkedin-posts", playbook)
        .unwrap()
        .id;
    apply(&fx.retro, &id, NOW + 1).unwrap();
    let path = fx
        .root
        .join(".nandi/agents/writer/skills/linkedin-posts.md");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), playbook);
    undo(&fx.retro, &id, NOW + 2).unwrap();
    assert!(!path.exists());
    assert_eq!(fx.head().0, tree_before);
}

#[test]
fn an_edit_since_is_never_overwritten() {
    let fx = Fixture::new();
    let after = WRITER.replace("Draft posts for the user.\n", SHORTER);
    let id = fx.propose("writer", "skill", &after).unwrap().id;
    let skill = fx.root.join(".nandi/agents/writer/skill.md");
    std::fs::write(&skill, WRITER.replace("Draft", "Write")).unwrap();
    assert!(apply(&fx.retro, &id, NOW + 1)
        .unwrap_err()
        .contains("changed since"));
    std::fs::write(&skill, WRITER).unwrap();
    apply(&fx.retro, &id, NOW + 1).unwrap();
    std::fs::write(&skill, format!("{after}\nMine.\n")).unwrap();
    assert!(undo(&fx.retro, &id, NOW + 2)
        .unwrap_err()
        .contains("changed since"));
    assert!(fx.skill().contains("Mine."), "the person's edit stays");
}

#[test]
fn refusing_drops_it_from_the_inbox() {
    let fx = Fixture::new();
    let after = WRITER.replace("Draft posts for the user.\n", SHORTER);
    let id = fx.propose("writer", "skill", &after).unwrap().id;
    refuse(&fx.retro, &id, NOW + 1).unwrap();
    assert!(items(&fx.retro, NOW + 2).unwrap().is_empty());
    assert!(apply(&fx.retro, &id, NOW + 2).is_err());
    assert_eq!(fx.skill(), WRITER);
}

/// Decisions on `writer`'s runs, `corrections` of `total` refused, at `at`
fn decide_runs(fx: &Fixture, prefix: &str, total: u32, corrections: u32, at: i64) {
    let conn = fx.retro.db.conn().unwrap();
    for i in 0..total {
        let status = if i < corrections {
            "rejected"
        } else {
            "approved"
        };
        conn.execute(
            "INSERT INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool, args,
                                    status, created_at, resolved_at)
             VALUES (?1, 'writer', 'r1', ?1, ?1, 'write_file', '{}', ?2, ?3, ?3)",
            rusqlite::params![format!("{prefix}{i}"), status, at],
        )
        .unwrap();
    }
}

#[test]
fn the_check_compares_rates_from_run_records_and_offers_the_undo() {
    let fx = Fixture::new();
    let after = WRITER.replace("Draft posts for the user.\n", SHORTER);
    let id = fx.propose("writer", "skill", &after).unwrap().id;
    let applied_at = NOW + 1;
    apply(&fx.retro, &id, applied_at).unwrap();
    // Before: 2 of 4 refused. After: 3 of 5, worse
    decide_runs(&fx, "before-", 4, 2, applied_at - DAY_MS);
    decide_runs(&fx, "after-", 4, 3, applied_at + DAY_MS);
    assert_eq!(
        refresh_checks(&fx.retro, applied_at + 2 * DAY_MS).unwrap(),
        0,
        "too early"
    );
    decide_runs(&fx, "later-", 1, 0, applied_at + 2 * DAY_MS);

    let later = applied_at + 10 * DAY_MS;
    let items = items(&fx.retro, later).unwrap();
    assert_eq!(items.len(), 1, "the note stays past the week");
    let note = &items[0];
    assert_eq!(note.verdict, Some(RetroVerdict::Worse));
    let rates = note.rates.unwrap();
    assert_eq!((rates.before.decisions, rates.before.corrections), (4, 2));
    assert_eq!((rates.after.decisions, rates.after.corrections), (5, 3));
    assert!(note.needs_note);
    // The verdict is set once
    decide_runs(&fx, "much-later-", 20, 0, later);
    assert_eq!(
        super::items(&fx.retro, later + 1).unwrap()[0].verdict,
        Some(RetroVerdict::Worse)
    );

    // Keep it: the note goes; Undo still works from history
    keep(&fx.retro, &id, later + 2).unwrap();
    assert!(super::items(&fx.retro, later + 3).unwrap().is_empty());
}

#[test]
fn a_change_that_helped_leaves_no_note() {
    let fx = Fixture::new();
    let after = WRITER.replace("Draft posts for the user.\n", SHORTER);
    let id = fx.propose("writer", "skill", &after).unwrap().id;
    let applied_at = NOW + 1;
    apply(&fx.retro, &id, applied_at).unwrap();
    decide_runs(&fx, "before-", 4, 3, applied_at - DAY_MS);
    decide_runs(&fx, "after-", 5, 1, applied_at + DAY_MS);
    assert_eq!(
        refresh_checks(&fx.retro, applied_at + 2 * DAY_MS).unwrap(),
        1
    );
    assert!(items(&fx.retro, applied_at + 10 * DAY_MS)
        .unwrap()
        .is_empty());
}

#[test]
fn the_tools_are_only_in_the_weekly_retro_run() {
    assert!(tool::offered(builtin_agents::DEFAULT_AGENT, Some(WEEKLY_RETRO)).len() == 3);
    assert!(tool::offered(builtin_agents::DEFAULT_AGENT, Some("morning-digest")).is_empty());
    assert!(tool::offered(builtin_agents::DEFAULT_AGENT, None).is_empty());
    assert!(tool::offered("writer", Some(WEEKLY_RETRO)).is_empty());
    let fx = Fixture::new();
    let week = tool::call(&fx.retro, "retro-run", tool::RUNS, &json!({}), NOW);
    assert!(!week.is_error, "{}", week.result);
    let week: serde_json::Value = serde_json::from_str(&week.result).unwrap();
    assert_eq!(week["proposalsLeft"], 3);
    assert_eq!(week["agents"][0]["agent"], "writer");
    assert!(week["note"].as_str().unwrap().contains("untrusted"));
    let read = tool::call(
        &fx.retro,
        "retro-run",
        tool::READ,
        &json!({ "agent": "writer", "target": "skill" }),
        NOW,
    );
    assert_eq!(read.result, WRITER);
}

#[test]
fn nandi_ships_the_retro_playbook_and_trigger() {
    let playbooks = builtin_agents::default_agent().shipped_files();
    let (_, retro) = playbooks
        .iter()
        .find(|(path, _)| path == "skills/weekly-retro.md")
        .expect("the playbook ships");
    let playbook =
        crate::services::domain::playbook::Playbook::from_md("weekly-retro", retro).unwrap();
    assert!(playbook.words() <= 900);
    for tool in [tool::RUNS, tool::READ, tool::PROPOSE] {
        assert!(playbook.body().contains(tool), "{tool}");
    }
    let (_, yaml) = builtin_agents::DEFAULT_TRIGGERS
        .iter()
        .find(|(id, _)| *id == WEEKLY_RETRO)
        .expect("the trigger ships");
    let trigger = crate::services::domain::Trigger::from_yaml(yaml).unwrap();
    assert_eq!(trigger.id().as_str(), WEEKLY_RETRO);
    assert!(trigger.input_template().unwrap().contains("weekly-retro"));
}
