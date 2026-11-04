//! `nandi_save_artifact`: Ask's chat saves what it built, so "build me an
//! agent" ends with an agent in `.nandi/agents/`, not a file the user has to
//! copy. One tool for every definition Ask can write: agents, workflows,
//! triggers, connectors, capabilities and policies.
//!
//! Every call waits for the user's OK ([`gate`]): the approval card names
//! what will be written. Writes go through the owners Ask's editor and
//! Automations use (`artifact_saver`, the trigger service, the connector
//! service), so validation, projections and events are the same. Secrets
//! never pass through here: triggers generate theirs, and connector
//! secrets are entered in Library › Browse › Tools (the file names the
//! keyring entry only).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::db::PlannedCall;
use crate::engine::ToolOutcome;
use crate::services::ask::ArtifactRef;
use crate::services::triggers::{self, schema::TriggerInput};
use crate::services::AppState;

use super::{validate, ArtifactSave, PushTarget};

pub const SAVE_ARTIFACT: &str = "nandi_save_artifact";

/// What `kind` can be, in the order the tool lists them
pub const KINDS: &[&str] = &[
    "agent",
    "workflow",
    "trigger",
    "connector",
    "capability",
    "policy",
    "instructions",
];

/// Saving always asks: it changes the user's definitions
pub fn gate(calls: &mut [PlannedCall]) {
    for call in calls.iter_mut().filter(|c| c.tool == SAVE_ARTIFACT) {
        call.requires_approval = true;
    }
}

fn ok(value: Value) -> ToolOutcome {
    ToolOutcome {
        result: value.to_string(),
        is_error: false,
    }
}

fn error(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: message.into(),
        is_error: true,
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The artifact a file-shaped save writes; `None` for triggers and
/// connectors, which aren't files Ask's editor opens
pub fn artifact_of(kind: &str, id: &str) -> Result<Option<ArtifactRef>, String> {
    let id = id.to_string();
    Ok(Some(match kind {
        "agent" => ArtifactRef::Agent(id),
        "workflow" => ArtifactRef::Workflow(id),
        "capability" => ArtifactRef::Capability(id),
        "policy" => ArtifactRef::Policy(id),
        "trigger" | "connector" | "instructions" => return Ok(None),
        other => {
            return Err(format!(
                "Unknown kind '{other}': use one of {}",
                KINDS.join(", ")
            ))
        }
    }))
}

/// Runs `nandi_save_artifact` (after the user approved it)
pub async fn call(state: &AppState, args: &Value) -> ToolOutcome {
    let Some(kind) = arg(args, "kind") else {
        return error(format!("kind is required: one of {}", KINDS.join(", ")));
    };
    match kind {
        "trigger" => save_trigger(state, args).await,
        "connector" => save_connector(state, args).await,
        "instructions" => save_instructions(state, args).await,
        _ => save_file(state, kind, args).await,
    }
}

/// Agents, workflows, capabilities and policies: checked like
/// `nandi_check_artifact`, then saved and committed like Ask's editor
async fn save_file(state: &AppState, kind: &str, args: &Value) -> ToolOutcome {
    let Some(content) = args.get("content").and_then(Value::as_str) else {
        return error("content is required: the complete file");
    };
    // A workflow's ID is its `id:`; the others are named by `id`
    let id = match (kind, arg(args, "id")) {
        (_, Some(id)) => id.to_string(),
        ("workflow", None) => match crate::services::domain::Workflow::from_yaml(content) {
            Ok(workflow) => workflow.id().to_string(),
            Err(e) => return error(format!("Not a valid workflow: {e}")),
        },
        _ => return error("id is required"),
    };
    let artifact = match artifact_of(kind, &id) {
        Ok(Some(artifact)) => artifact,
        Ok(None) => return error(format!("kind '{kind}' takes no content")),
        Err(e) => return error(e),
    };
    if matches!(
        artifact,
        ArtifactRef::Agent(_) | ArtifactRef::Workflow(_) | ArtifactRef::Policy(_)
    ) {
        let checked = validate::validate(
            &artifact,
            content,
            &super::tools::validation_facts(state).await,
        );
        if !checked.ok {
            return ok(json!({
                "saved": false,
                "errors": checked.errors,
                "warnings": checked.warnings,
                "next": "Fix the errors, show the file again, then save it.",
            }));
        }
    }
    let existed = exists(state, &artifact);
    let save = ArtifactSave {
        artifact: artifact.clone(),
        content: content.to_string(),
        commit_message: arg(args, "message").unwrap_or_default().to_string(),
        push_target: PushTarget::LocalOnly,
    };
    match super::save_artifact(state, save).await {
        Ok(result) => ok(json!({
            "saved": true,
            "created": !existed,
            "kind": kind,
            "id": id,
            "commit": result.commit,
            "warnings": result.warnings,
        })),
        Err(e) => error(e),
    }
}

fn exists(state: &AppState, artifact: &ArtifactRef) -> bool {
    match artifact {
        ArtifactRef::Agent(id) => state.store().read(|repo| repo.agent(id).is_some()),
        ArtifactRef::Workflow(id) => state.store().read(|repo| repo.workflow(id).is_some()),
        ArtifactRef::Capability(id) => state
            .capabilities()
            .is_some_and(|service| service.repo().load(id).is_ok()),
        ArtifactRef::Policy(id) => state.store().repos().policies.path(id).is_file(),
        ArtifactRef::Gateway => true,
    }
}

/// `trigger`: a `TriggerInput` (as Automations › Triggers sends it).
/// Updates the trigger when its `id` exists, else creates it.
async fn save_trigger(state: &AppState, args: &Value) -> ToolOutcome {
    let Some(trigger) = args.get("trigger").filter(|t| t.is_object()) else {
        return error("trigger is required for kind 'trigger'");
    };
    let input: TriggerInput = match serde_json::from_value(trigger.clone()) {
        Ok(input) => input,
        Err(e) => return error(format!("Unreadable trigger: {e}")),
    };
    let existing = match &input.id {
        Some(id) => triggers::get_trigger(state, id.clone()).await.ok(),
        None => None,
    };
    let saved = match &existing {
        Some(existing) => triggers::update_trigger(state, existing.id.clone(), input).await,
        None => triggers::create_trigger(state, input).await,
    };
    match saved {
        Ok(saved) => ok(json!({
            "saved": true,
            "created": existing.is_none(),
            "kind": "trigger",
            "id": saved.trigger.id,
            "enabled": saved.trigger.enabled,
            // The secret itself stays out of the conversation
            "secret": saved.secret.is_some().then_some(
                "A webhook secret was generated: the user sees it with Rotate secret on the trigger's page in Automations › Triggers"
            ),
        })),
        Err(e) => error(e),
    }
}

/// `instructions`: only agent `id`'s instructions.md (`content`), the
/// user's rules. An installed agent's go to its override: it stays
/// read-only, and updates keep them.
async fn save_instructions(state: &AppState, args: &Value) -> ToolOutcome {
    let Some(id) = arg(args, "id") else {
        return error("id is required: the agent's folder name");
    };
    let Some(content) = args.get("content").and_then(Value::as_str) else {
        return error("content is required: the whole instructions.md");
    };
    let overrides = state
        .store()
        .repos()
        .agents
        .locate(id)
        .is_some_and(|place| place.origin.is_vendored());
    match crate::services::repo_service::save_agent_instructions(state, id, content.to_string())
        .await
    {
        Ok(()) => ok(json!({
            "saved": true,
            "kind": "instructions",
            "id": id,
            "next": if overrides {
                "Saved as the user's own rules for this installed agent: updates keep them, and Settings › Your rules can reset them."
            } else {
                "Saved in the agent's instructions.md."
            },
        })),
        Err(e) => error(e),
    }
}

/// `connector`: a built-in by `id` (installed as is), or `kind` plus
/// `config` (and `secret_refs` naming keyring entries) for an MCP server
async fn save_connector(state: &AppState, args: &Value) -> ToolOutcome {
    use crate::services::connectors;

    let Some(id) = arg(args, "id") else {
        return error("id is required for kind 'connector'");
    };
    let connector = args.get("connector").filter(|c| c.is_object());
    let config = match connector.and_then(|c| c.get("config")) {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => {
            match connectors::yaml_config(map.clone().into_iter().collect()) {
                Ok(config) => Some(config),
                Err(e) => return error(format!("Unreadable config: {e}")),
            }
        }
        Some(_) => return error("connector.config must be an object"),
    };
    let secret_refs: BTreeMap<String, String> =
        match connector.and_then(|c| c.get("secret_refs")) {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(refs) => match serde_json::from_value(refs.clone()) {
                Ok(refs) => refs,
                Err(_) => {
                    return error(
                        "connector.secret_refs maps a name to a keyring entry (`nandi.connector.<id>.<name>`), never to a secret",
                    )
                }
            },
        };
    let kind = connector
        .and_then(|c| c.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let result = connectors::declare_connector(
        state,
        connectors::ConnectorDeclaration {
            id: id.to_string(),
            kind: kind.map(str::to_string),
            config,
            secret_refs,
        },
    )
    .await;
    match result {
        Ok(info) => ok(json!({
            "saved": true,
            "kind": "connector",
            "id": info.id,
            "tools": info.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            "setup": info.setup_hint,
            "secrets": info.secrets.iter().filter(|s| !s.is_set).map(|s| s.key.clone()).collect::<Vec<_>>(),
            "next": format!(
                "An agent uses it once its skill.md lists `{id}` under `connectors`. Missing secrets and sign-ins are set in Library › Browse › Tools."
            ),
        })),
        Err(e) => error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_save_waits_for_approval() {
        let mut calls = vec![
            PlannedCall {
                id: "1".into(),
                tool: SAVE_ARTIFACT.into(),
                args: json!({ "kind": "agent", "id": "writer", "content": "" }),
                requires_approval: false,
            },
            PlannedCall {
                id: "2".into(),
                tool: "nandi_list_agents".into(),
                args: json!({}),
                requires_approval: false,
            },
        ];
        gate(&mut calls);
        assert!(calls[0].requires_approval);
        assert!(!calls[1].requires_approval);
    }

    #[test]
    fn kinds_map_to_what_saves_them() {
        assert_eq!(
            artifact_of("agent", "writer").unwrap(),
            Some(ArtifactRef::Agent("writer".into()))
        );
        assert_eq!(artifact_of("trigger", "t").unwrap(), None);
        assert_eq!(artifact_of("connector", "c").unwrap(), None);
        assert_eq!(artifact_of("instructions", "c").unwrap(), None);
        assert!(artifact_of("gateway", "x").is_err());
        for kind in KINDS {
            assert!(artifact_of(kind, "x").is_ok(), "{kind}");
        }
    }
}
