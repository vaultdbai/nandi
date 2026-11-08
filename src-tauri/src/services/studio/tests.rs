use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::services::auth;
use crate::services::remote::{configure_remote, git_ops};

const CAP: &str = "---\nid: code-review.python\nname: Python review\ndescription: Reviews Python\nversion: 1.2.3\nowner:\n  kind: user\n  id: ada\n  display_name: Ada\npolicy:\n  sla_seconds: 300\n---\n\nReview it.\n";
const WORKFLOW: &str =
    "id: daily\nname: Daily\nsteps:\n  - id: one\n    agent: writer\n    input: hi\n";
const SKILL: &str = "---\nname: Writer\ndescription: Writes\n---\nWrite clearly.\n";

/// A repo under git with an agent, a workflow, and a capability
async fn state() -> AppState {
    recorded_state(vec![]).await
}

/// [`state`], with its events going to `sinks`
async fn recorded_state(sinks: Vec<Arc<dyn crate::services::events::EventSink>>) -> AppState {
    let state = AppState::for_tests(sinks);
    auth::set_display_name(&state, "Alice Chen".into())
        .await
        .unwrap();
    let layout = state.store().layout().clone();
    let skill = layout.skill_path("writer");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, SKILL).unwrap();
    std::fs::create_dir_all(&layout.workflows_dir).unwrap();
    std::fs::write(layout.workflows_dir.join("daily.yaml"), WORKFLOW).unwrap();
    std::fs::create_dir_all(layout.capabilities_dir()).unwrap();
    std::fs::write(layout.capability_path("code-review.python"), CAP).unwrap();
    let repo = git_ops::open_or_init(&layout.root).unwrap();
    crate::services::repo_bootstrap::commit_all(&repo, "Setup", Some("alice")).unwrap();
    state.store().load().unwrap();
    state.capabilities().unwrap().rebuild().await.unwrap();
    state
}

fn head(root: &Path) -> (String, git2::Oid) {
    let repo = git2::Repository::open(root).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    (commit.message().unwrap().to_string(), commit.id())
}

/// A file as committed at `HEAD`
fn committed(root: &Path, path: &str) -> Option<String> {
    let repo = git2::Repository::open(root).unwrap();
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).unwrap();
    Some(String::from_utf8(blob.content().to_vec()).unwrap())
}

fn save(artifact: ArtifactRef, content: &str, target: PushTarget) -> ArtifactSave {
    ArtifactSave {
        artifact,
        content: content.to_string(),
        commit_message: "Studio edit".into(),
        push_target: target,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_artifact_type_loads() {
    let state = state().await;

    let capability = load_artifact(&state, ArtifactRef::Capability("code-review.python".into()))
        .await
        .unwrap();
    assert_eq!(capability.content, CAP);
    assert_eq!(capability.parsed["frontmatter"]["version"], "1.2.3");
    assert_eq!(capability.parsed["body"], "Review it.");
    assert!(capability.path.ends_with("code-review.python.yaml"));

    let agent = load_artifact(&state, ArtifactRef::Agent("writer".into()))
        .await
        .unwrap();
    assert_eq!(agent.content, SKILL);
    assert_eq!(agent.parsed["frontmatter"]["name"], "Writer");
    assert_eq!(agent.parsed["body"], "Write clearly.");

    let workflow = load_artifact(&state, ArtifactRef::Workflow("daily".into()))
        .await
        .unwrap();
    assert_eq!(
        workflow.parsed["frontmatter"]["steps"][0]["agent"],
        "writer"
    );
    assert_eq!(workflow.parsed["body"], "");

    // Not there yet: starter templates
    let policy = load_artifact(&state, ArtifactRef::Policy("spending".into()))
        .await
        .unwrap();
    assert_eq!(policy.parsed["frontmatter"]["name"], "spending");
    assert_eq!(
        policy.parsed["frontmatter"]["applies_to"],
        serde_json::json!([])
    );
    assert!(
        policy.path.ends_with("policies/spending.md")
            || policy.path.ends_with("policies\\spending.md")
    );
    let new_capability = load_artifact(&state, ArtifactRef::Capability("translate".into()))
        .await
        .unwrap();
    assert_eq!(new_capability.parsed["frontmatter"]["version"], "0.1.0");
    let gateway = load_artifact(&state, ArtifactRef::Gateway).await.unwrap();
    assert!(gateway.path.ends_with("NANDI.md"));
    assert!(gateway.parsed["frontmatter"].is_object());

    // Unknown or invalid
    assert!(load_artifact(&state, ArtifactRef::Agent("ghost".into()))
        .await
        .is_err());
    assert!(load_artifact(&state, ArtifactRef::Policy("../x".into()))
        .await
        .is_err());
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_save_commits_only_that_file() {
    let state = state().await;
    let root = state.store().layout().root.clone();
    let (_, before) = head(&root);
    // Something else changed and staged: not part of the commit
    std::fs::write(root.join("notes.txt"), "unrelated").unwrap();

    let edited = WORKFLOW.replace("input: hi", "input: hello");
    let result = save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            &edited,
            PushTarget::LocalOnly,
        ),
    )
    .await
    .unwrap();
    let (message, after) = head(&root);
    assert_eq!(result.commit, Some(after.to_string()));
    assert_ne!(before, after);
    assert_eq!(message, "Studio edit");
    let path = state
        .store()
        .layout()
        .relative(&state.store().layout().workflows_dir.join("daily.yaml"))
        .unwrap();
    assert_eq!(committed(&root, &path).as_deref(), Some(edited.as_str()));
    assert_eq!(committed(&root, "notes.txt"), None);
    assert!(result.pushed_branch.is_none());
    assert!(result.warnings.is_empty());
    // The projection follows the file
    let workflow = state.workflows().get_workflow("daily").await.unwrap();
    assert_eq!(workflow.steps[0].input, "hello");

    // Saving the same text again commits nothing
    let again = save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            &edited,
            PushTarget::LocalOnly,
        ),
    )
    .await
    .unwrap();
    assert_eq!(again.commit, None);
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_and_policies_save_through_their_repositories() {
    let state = state().await;
    let root = state.store().layout().root.clone();
    let edited = SKILL.replace("name: Writer", "name: Scribe");
    save_artifact(
        &state,
        save(
            ArtifactRef::Agent("writer".into()),
            &edited,
            PushTarget::LocalOnly,
        ),
    )
    .await
    .unwrap();
    let name = state
        .store()
        .read(|repo| repo.agent("writer").map(|a| a.name().to_string()));
    assert_eq!(name.as_deref(), Some("Scribe"));
    assert!(committed(&root, ".nandi/agents/writer/skill.md")
        .unwrap()
        .contains("name: Scribe"));

    let policy = "---\nname: spending\ndescription: Spending limits\n---\n\nAsk before spending.\n";
    let result = save_artifact(
        &state,
        save(
            ArtifactRef::Policy("spending".into()),
            policy,
            PushTarget::LocalOnly,
        ),
    )
    .await
    .unwrap();
    assert!(result.commit.is_some());
    assert_eq!(
        committed(&root, ".nandi/policies/spending.md").as_deref(),
        Some(policy)
    );

    // A policy the enforcer would refuse isn't saved
    let error = save_artifact(
        &state,
        save(
            ArtifactRef::Policy("no-target".into()),
            "---\ntools: { deny: [shell] }\n---\n",
            PushTarget::LocalOnly,
        ),
    )
    .await
    .unwrap_err();
    assert!(error.contains("applies_to"), "{error}");
    assert!(committed(&root, ".nandi/policies/no-target.md").is_none());

    // Invalid text is refused before anything is written
    assert!(save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            "id: daily\nsteps: nope",
            PushTarget::LocalOnly
        ),
    )
    .await
    .is_err());
    assert!(save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            &WORKFLOW.replace("id: daily", "id: other"),
            PushTarget::LocalOnly
        ),
    )
    .await
    .unwrap_err()
    .contains("can't change"));
    state.shutdown();
}

/// `nandi_save_artifact` as the chat calls it, after approval
async fn chat_saves(state: &AppState, args: serde_json::Value) -> serde_json::Value {
    let outcome = super::tools::call(state, super::save::SAVE_ARTIFACT, &args).await;
    assert!(!outcome.is_error, "{}", outcome.result);
    serde_json::from_str(&outcome.result).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_policy_saved_from_the_chat_is_announced() {
    let recorder = Arc::new(crate::services::events::tests::RecordingSink::default());
    let state = recorded_state(vec![recorder.clone()]).await;
    let policy = "---
applies_to: ['*']
tools: { deny: [shell] }
---
";
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "policy", "id": "no-shell", "content": policy }),
    )
    .await;
    assert_eq!(saved["saved"], true);
    // The same event Ask's editor sends, so every agent's policies refresh
    let announced = recorder
        .take()
        .into_iter()
        .find(|(name, _)| *name == STUDIO_ARTIFACT_SAVED)
        .map(|(_, payload)| payload)
        .expect("studio://artifact_saved");
    assert_eq!(
        announced["result"]["artifact"],
        serde_json::json!({ "kind": "policy", "id": "no-shell" })
    );
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_creates_what_it_builds() {
    let state = state().await;
    let root = state.store().layout().root.clone();

    // A new agent, then a change to it
    let skill = "---\nname: Inbox\ndescription: Summarises the inbox\nconnectors: [filesystem]\n---\nSummarise.\n";
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "agent", "id": "inbox", "content": skill }),
    )
    .await;
    assert_eq!(
        (saved["saved"].as_bool(), saved["created"].as_bool()),
        (Some(true), Some(true))
    );
    assert!(committed(&root, ".nandi/agents/inbox/skill.md")
        .unwrap()
        .contains("connectors"));
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "agent", "id": "inbox", "content": skill.replace("Summarise.", "Summarise briefly.") }),
    )
    .await;
    assert_eq!(saved["created"], false);
    assert!(state
        .store()
        .read(|repo| repo.agent("inbox").map(|a| a.to_skill_md()))
        .unwrap()
        .contains("briefly"));

    // A failing check saves nothing and says why
    let refused = chat_saves(
        &state,
        serde_json::json!({ "kind": "agent", "id": "bad", "content": "---\nmodel: sonnet\n---\nx\n" }),
    )
    .await;
    assert_eq!(refused["saved"], false);
    assert!(state.store().read(|repo| repo.agent("bad").is_none()));

    // A new workflow, named by its `id:`
    let workflow =
        "id: morning\nname: Morning\nsteps:\n  - id: one\n    agent: inbox\n    input: go\n";
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "workflow", "content": workflow }),
    )
    .await;
    assert_eq!(
        (saved["id"].as_str(), saved["created"].as_bool()),
        (Some("morning"), Some(true))
    );
    assert_eq!(
        committed(&root, ".nandi/workflows/morning.yaml").as_deref(),
        Some(workflow)
    );

    // A built-in connector, then an MCP server; secrets are refused
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "connector", "id": "web_search" }),
    )
    .await;
    assert_eq!(saved["id"], "web_search");
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "connector", "id": "notes", "connector": {
            "kind": "mcp",
            "config": { "name": "Notes", "command": "npx", "args": ["-y", "server-memory"] },
            "secret_refs": { "NOTES_TOKEN": "nandi.connector.notes.token" }
        }}),
    )
    .await;
    assert_eq!(saved["secrets"], serde_json::json!(["NOTES_TOKEN"]));
    let connectors = state.store().read(|repo| repo.connectors().clone());
    assert!(connectors.get("web_search").is_some() && connectors.get("notes").is_some());
    let leak = super::tools::call(
        &state,
        super::save::SAVE_ARTIFACT,
        &serde_json::json!({ "kind": "connector", "id": "leaky", "connector": {
            "kind": "mcp", "config": { "command": "x", "api_key": "sk-123" }
        }}),
    )
    .await;
    assert!(leak.is_error, "a key in config is refused");
    assert!(state
        .store()
        .read(|repo| repo.connectors().get("leaky").is_none()));

    // A trigger, created then updated by its ID
    let trigger = serde_json::json!({
        "id": "weekday-inbox", "name": "Weekday inbox", "sourceType": "cron",
        "sourceConfig": { "schedule": "0 9 * * 1-5" },
        "targetType": "workflow", "targetId": "morning"
    });
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "trigger", "trigger": trigger }),
    )
    .await;
    assert_eq!(
        (saved["id"].as_str(), saved["created"].as_bool()),
        (Some("weekday-inbox"), Some(true))
    );
    let mut renamed = trigger.clone();
    renamed["name"] = "Inbox on weekdays".into();
    let saved = chat_saves(
        &state,
        serde_json::json!({ "kind": "trigger", "trigger": renamed }),
    )
    .await;
    assert_eq!(saved["created"], false);
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capability_saves_must_bump_the_version_enough() {
    let state = state().await;
    let artifact = ArtifactRef::Capability("code-review.python".into());
    let faster = CAP.replace("sla_seconds: 300", "sla_seconds: 60");

    let missing = save_artifact(
        &state,
        save(artifact.clone(), &faster, PushTarget::LocalOnly),
    )
    .await
    .unwrap_err();
    assert!(
        missing.contains("needs a minor version bump: set version to 1.3.0"),
        "{missing}"
    );
    let too_small = faster.replace("version: 1.2.3", "version: 1.2.4");
    assert!(save_artifact(
        &state,
        save(artifact.clone(), &too_small, PushTarget::LocalOnly)
    )
    .await
    .unwrap_err()
    .contains("needs a minor version bump"));
    let down = CAP.replace("version: 1.2.3", "version: 1.0.0");
    assert!(
        save_artifact(&state, save(artifact.clone(), &down, PushTarget::LocalOnly))
            .await
            .unwrap_err()
            .contains("can't go down")
    );

    let bumped = faster.replace("version: 1.2.3", "version: 1.3.0");
    let result = save_artifact(
        &state,
        save(artifact.clone(), &bumped, PushTarget::LocalOnly),
    )
    .await
    .unwrap();
    assert!(result.commit.is_some());
    let saved = state
        .capabilities()
        .unwrap()
        .get("code-review.python")
        .await
        .unwrap();
    assert_eq!(saved.version, "1.3.0");
    assert_eq!(saved.policy.sla_seconds, 60);
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_targets_follow_the_remote() {
    let state = state().await;
    let targets = list_push_targets(&state).await.unwrap();
    assert_eq!(targets.len(), 3);
    assert!(targets[0].available);
    assert!(!targets[1].available && targets[1].reason.is_some());
    assert!(!targets[2].available);

    // No remote: refused, and nothing is written
    let edited = WORKFLOW.replace("input: hi", "input: hey");
    let error = save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            &edited,
            PushTarget::RemoteBranch,
        ),
    )
    .await
    .unwrap_err();
    assert!(error.contains("No remote is configured"));
    let on_disk =
        std::fs::read_to_string(state.store().layout().workflows_dir.join("daily.yaml")).unwrap();
    assert_eq!(on_disk, WORKFLOW);

    // A local bare repo stands in for the remote
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("remote.git");
    let mut options = git2::RepositoryInitOptions::new();
    options.bare(true).initial_head("main");
    git2::Repository::init_opts(&bare, &options).unwrap();
    configure_remote(
        &state,
        bare.to_string_lossy().into_owned(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let targets = list_push_targets(&state).await.unwrap();
    assert!(targets[1].available);
    assert_eq!(targets[1].branch.as_deref(), Some("nandi/alice-chen"));

    let result = save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            &edited,
            PushTarget::RemoteBranch,
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.pushed_branch.as_deref(), Some("nandi/alice-chen"));
    let remote = git2::Repository::open_bare(&bare).unwrap();
    let pushed = remote
        .find_reference("refs/heads/nandi/alice-chen")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    assert_eq!(Some(pushed.id().to_string()), result.commit);
    assert!(
        remote.find_reference("refs/heads/main").is_err(),
        "never main"
    );

    // With a pull request: pushed, and the PR part says it isn't there yet
    let again = WORKFLOW.replace("input: hi", "input: howdy");
    let result = save_artifact(
        &state,
        save(
            ArtifactRef::Workflow("daily".into()),
            &again,
            PushTarget::RemoteBranchAndPr,
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.pushed_branch.as_deref(), Some("nandi/alice-chen"));
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("pull request isn't implemented")));
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Chat starts work (`start`): approval first, then a Request on Home
// ---------------------------------------------------------------------------

mod chat_starts_work {
    use std::time::Duration;

    use serde_json::json;

    use crate::db;
    use crate::model::{ApprovalDecision, PendingApproval};
    use crate::services::chat;
    use crate::services::work::{self, WorkKind};
    use crate::services::AppState;

    /// Studio replies by starting a Request for the `daily` workflow (plus
    /// a made-up agent the gate must drop), then says it's done
    async fn studio_starts_daily(state: &AppState) {
        let script = json!({
            "agent": [{ "text": "ok" }],
            "studio": [
                {
                    "text": "Starting it.",
                    "toolCalls": [{
                        "tool": super::super::start::START_REQUEST,
                        "args": {
                            "text": "Run the daily workflow",
                            "plan": { "steps": [
                                { "kind": "workflow", "target": "daily", "description": "Run Daily" },
                                { "kind": "localAgent", "target": "ghost", "description": "Invented" }
                            ] }
                        },
                        "result": ""
                    }]
                },
                { "text": "Done." }
            ],
            "tokenDelayMs": 0
        });
        state
            .db()
            .call(move |conn| {
                db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
                db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
            })
            .await
            .unwrap();
    }

    async fn pending(state: &AppState) -> Vec<PendingApproval> {
        state
            .db()
            .call(|conn| db::pending_approvals(conn, None))
            .await
            .unwrap()
    }

    async fn requests(state: &AppState) -> usize {
        work::list_work(state, None)
            .await
            .unwrap()
            .iter()
            .filter(|item| item.kind == WorkKind::Request)
            .count()
    }

    async fn wait_for<F, Fut>(what: &str, mut check: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..500 {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// Sends the ask and waits for the approval card, which shows the
    /// checked plan
    async fn ask_and_wait(state: &AppState) -> PendingApproval {
        studio_starts_daily(state).await;
        let conversation = chat::create_studio_conversation(state).await.unwrap();
        chat::send_studio_message(
            state,
            conversation.id,
            "Run the daily workflow".into(),
            vec![],
            None,
        )
        .await
        .unwrap();
        wait_for("the approval", || async {
            !pending(state).await.is_empty()
        })
        .await;
        let approval = pending(state).await.remove(0);
        assert_eq!(approval.tool, super::super::start::START_REQUEST);
        let targets: Vec<&str> = approval.args["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["target"].as_str().unwrap())
            .collect();
        assert_eq!(
            targets,
            ["daily"],
            "unknown targets are dropped before approval"
        );
        approval
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn declining_starts_nothing() {
        let state = super::state().await;
        let approval = ask_and_wait(&state).await;
        assert_eq!(requests(&state).await, 0, "nothing starts before approval");

        chat::approve_action(&state, None, approval.id, ApprovalDecision::Rejected)
            .await
            .unwrap();
        wait_for("the run to finish", || async {
            pending(&state).await.is_empty()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(requests(&state).await, 0);
        state.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_approved_plan_becomes_a_request_on_home() {
        let state = super::state().await;
        let approval = ask_and_wait(&state).await;

        chat::approve_action(&state, None, approval.id, ApprovalDecision::Approved)
            .await
            .unwrap();
        wait_for("the Request", || async { requests(&state).await == 1 }).await;
        state.shutdown();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_reads_the_users_notes_and_nothing_outside_them() {
    use crate::services::connectors::{self, folders};
    use crate::services::notes::NOTES_FOLDER_ID;

    let state = AppState::for_tests(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let notes = tmp.path().join("notes");
    std::fs::create_dir_all(notes.join("runbooks")).unwrap();
    std::fs::write(
        notes.join("deadlines.md"),
        "# Deadlines\n\nQ4 report: Dec 1\n",
    )
    .unwrap();
    std::fs::write(notes.join("runbooks/etl.md"), "# ETL\n").unwrap();
    std::fs::write(tmp.path().join("secret.md"), "outside").unwrap();
    connectors::share_folder(
        &state,
        folders::SharedFolder {
            id: NOTES_FOLDER_ID.into(),
            path: notes.to_string_lossy().to_string(),
            access: folders::Access::Read,
        },
    )
    .await
    .unwrap();

    let call = |name: &'static str, args: serde_json::Value| {
        let state = state.clone();
        async move { super::tools::call(&state, name, &args).await }
    };

    let listed = call(super::tools::LIST_NOTES, serde_json::json!({})).await;
    assert!(!listed.is_error, "{}", listed.result);
    let listed: serde_json::Value = serde_json::from_str(&listed.result).unwrap();
    assert_eq!(listed["count"], 2);
    assert_eq!(listed["notes"][0]["name"], "deadlines.md");
    assert_eq!(listed["notes"][0]["title"], "Deadlines");
    assert_eq!(listed["notes"][1]["name"], "runbooks/etl.md");

    let read = call(
        super::tools::READ_NOTE,
        serde_json::json!({ "name": "deadlines.md" }),
    )
    .await;
    assert!(!read.is_error, "{}", read.result);
    assert!(read.result.contains("Q4 report: Dec 1"));

    for name in ["../secret.md", "missing.md"] {
        let refused = call(super::tools::READ_NOTE, serde_json::json!({ "name": name })).await;
        assert!(refused.is_error, "{name}: {}", refused.result);
        assert!(!refused.result.contains("outside"));
    }
    let unnamed = call(super::tools::READ_NOTE, serde_json::json!({})).await;
    assert!(unnamed.is_error);
    state.shutdown();
}
