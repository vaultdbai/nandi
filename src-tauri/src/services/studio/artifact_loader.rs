//! Loads an artifact for editing: where its file lives, the raw text, and
//! a parsed view (`{ frontmatter, body }`) for Studio's forms. A
//! capability, policy, or NANDI.md that doesn't exist yet loads as a
//! starter template. Policies are `.nandi/policies/<id>.md`.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::{ArtifactLoad, StudioError};
use crate::services::ask::ArtifactRef;
use crate::services::capability::definition::valid_id;
use crate::services::domain::WorkflowId;
use crate::services::gateway::manifest;
use crate::services::repos::layout::SKILL_FILE;
use crate::services::AppState;

pub fn policies_dir(state: &AppState) -> PathBuf {
    state.store().repos().policies.dir()
}

/// `^[a-z0-9][a-z0-9-]*$`
pub fn valid_policy_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The artifact's file. Agents and workflows must exist; capabilities,
/// policies, and NANDI.md may not exist yet.
pub fn path_of(state: &AppState, artifact: &ArtifactRef) -> Result<PathBuf, StudioError> {
    let layout = state.store().layout();
    match artifact {
        ArtifactRef::Capability(id) => {
            if !valid_id(id) {
                return Err(format!("'{id}' isn't a valid capability ID"));
            }
            Ok(layout.capability_path(id))
        }
        ArtifactRef::Agent(id) => state
            .store()
            .repos()
            .agents
            .locate(id)
            .map(|place| place.dir.join(SKILL_FILE))
            .ok_or_else(|| format!("Agent '{id}' not found")),
        ArtifactRef::Workflow(id) => {
            let workflow_id = WorkflowId::parse(id).map_err(|e| e.to_string())?;
            state
                .store()
                .repos()
                .workflows
                .locate(&workflow_id)
                .map(|(path, _)| path)
                .ok_or_else(|| format!("Workflow '{id}' not found"))
        }
        ArtifactRef::Policy(id) => {
            if !valid_policy_id(id) {
                return Err(format!(
                    "'{id}' isn't a valid policy ID (lowercase letters, digits, and dashes)"
                ));
            }
            Ok(state.store().repos().policies.path(id))
        }
        ArtifactRef::Gateway => Ok(manifest::manifest_path(&layout.root)),
    }
}

/// The artifact's file as it is on disk. A capability only a subscribed
/// registry defines reads as the registry's file (saved, it overrides it).
pub fn read_raw(state: &AppState, artifact: &ArtifactRef) -> Result<String, StudioError> {
    let path = path_of(state, artifact)?;
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            registry_text(state, artifact).ok_or_else(|| format!("{}: {e}", path.display()))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The subscribed registry's file for a capability with no local file
fn registry_text(state: &AppState, artifact: &ArtifactRef) -> Option<String> {
    let ArtifactRef::Capability(id) = artifact else {
        return None;
    };
    state.capability_registries().source_text(id)
}

/// The file (or the registry's, or a starter template), its parsed view,
/// and its path
pub fn load(state: &AppState, artifact: &ArtifactRef) -> Result<ArtifactLoad, StudioError> {
    let path = path_of(state, artifact)?;
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            registry_text(state, artifact).unwrap_or_else(|| template(artifact))
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let parsed = parse(artifact, &content)?;
    Ok(ArtifactLoad {
        artifact: artifact.clone(),
        content,
        parsed,
        path,
    })
}

/// `{ frontmatter, body }`: Markdown files split at their frontmatter;
/// YAML files (workflows, plain capability files) are all frontmatter
pub fn parse(artifact: &ArtifactRef, content: &str) -> Result<Value, StudioError> {
    let yaml_file = match artifact {
        ArtifactRef::Workflow(_) => true,
        ArtifactRef::Capability(_) => !content.trim_start().starts_with("---"),
        _ => false,
    };
    if yaml_file {
        let value: Value = if content.trim().is_empty() {
            json!({})
        } else {
            serde_yaml::from_str(content).map_err(|e| format!("Invalid YAML: {e}"))?
        };
        return Ok(json!({ "frontmatter": value, "body": "" }));
    }
    let (frontmatter, body) = frontmatter(content)?;
    Ok(json!({ "frontmatter": frontmatter, "body": body.trim() }))
}

/// A Markdown file's YAML frontmatter (`{}` without one) and body
pub fn frontmatter(text: &str) -> Result<(Value, String), StudioError> {
    let document =
        manifest::split(text).map_err(|e| e.trim_start_matches("NANDI.md: ").to_string())?;
    let frontmatter = serde_json::to_value(&document.frontmatter)
        .map_err(|e| format!("The frontmatter can't be read: {e}"))?;
    Ok((frontmatter, document.body))
}

/// A new artifact's starting file
fn template(artifact: &ArtifactRef) -> String {
    match artifact {
        ArtifactRef::Capability(id) => format!(
            "---\nid: {id}\nname: {id}\ndescription: What this capability does\nversion: 0.1.0\n\
             owner:\n  kind: user\n  id: ''\n  display_name: ''\npolicy:\n  visibility: private\n  \
             sla_seconds: 300\n  approval_required: false\ntags: []\n---\n\n\
             How a provider should handle a request.\n"
        ),
        ArtifactRef::Policy(id) => format!(
            "---\nname: {id}\ndescription: What this policy is for\napplies_to: []\n---\n\n\
             The rule, in plain words.\n"
        ),
        ArtifactRef::Gateway => manifest::render_default(""),
        // Agents and workflows must exist (`path_of`)
        ArtifactRef::Agent(_) | ArtifactRef::Workflow(_) => String::new(),
    }
}

/// Every policy file as `(id, text)`, by ID
pub fn policy_files(state: &AppState) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(policies_dir(state)) else {
        return Vec::new();
    };
    let mut files: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_name()?.to_str()?.strip_suffix(".md")?.to_string();
            if !valid_policy_id(&id) {
                return None;
            }
            Some((id, std::fs::read_to_string(&path).ok()?))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}
