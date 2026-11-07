//! Policies applied to an agent's run config.
//!
//! `RepoStore::skill` sends every `SkillConfig` through [`narrow`], so the
//! orchestrator (tools, approvals), the egress proxy (network, AWS) and the
//! runtime container (shared folders) all see what the agent may do once
//! every applying policy in `.nandi/policies/` has narrowed its skill.md
//! (`domain::policy`). A broken policy that may cover the agent, or a model
//! a policy refuses, stops it: `skill` returns the reason.
//!
//! Policies are read from disk on every `skill`, so an edit applies to the
//! next model call; the file watcher reports changes ([`check_file`]).

use serde::Serialize;
use specta::Type;

use crate::services::agents::SkillConfig;
use crate::services::connectors::folders::{Access, SharedFolder};
use crate::services::domain::capability as domain_caps;
use crate::services::domain::policy::{
    Effective, FileMode, FolderRequest, Outcome, Policy, PolicySet, PolicySummary, Request, Subject,
};
use crate::services::domain::Agent;
use crate::services::egress::capabilities::{
    AwsCapabilities, AwsMode, AwsServiceGrant, Capabilities, HostRule, NetworkCapabilities,
};
use crate::services::repo_store::RepoStore;

/// `skill`, narrowed by `policies` for `agent`
pub fn narrow(
    agent: &Agent,
    skill: SkillConfig,
    policies: &PolicySet,
) -> Result<SkillConfig, String> {
    let effective = evaluate(agent, &skill, policies)?;
    Ok(apply(skill, &effective))
}

/// What `policies` leave `agent`, or why it can't run
fn evaluate(agent: &Agent, skill: &SkillConfig, policies: &PolicySet) -> Result<Effective, String> {
    let request = request(agent, skill);
    let tags = agent.tags();
    // Runs don't say which workflow they're a step of, and the egress proxy
    // sees agents, not runs: `workflow:` targets match nothing yet
    let subject = Subject {
        agent: &skill.id,
        tags: &tags,
        workflow: None,
    };
    let effective = policies.effective(&request, &subject)?;
    // A coding agent's model is its CLI's (`runtime_config.model`); the
    // gateway checks those
    let model = effective.allows_model(&skill.model);
    if let (true, Err(by)) = (agent.runtime().is_native(), model) {
        return Err(format!(
            "The policy '{by}' doesn't allow the model {} for agent '{}'",
            skill.model, skill.id
        ));
    }
    Ok(effective)
}

/// The policies over one agent, for its Permissions panel
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicies {
    /// The policies that apply, by id
    pub policies: Vec<String>,
    /// Why the agent can't run (a broken policy, a refused model)
    pub blocked: Option<String>,
    /// What each policy changed
    pub changes: Vec<PolicyChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PolicyChange {
    /// `tool shell`, `folder reports`, `host api.github.com`, ...
    pub item: String,
    pub outcome: ChangeOutcome,
    /// For `narrowed`: what's left (`read only`, `api.github.com`)
    pub detail: Option<String>,
    /// The policy's id
    pub by: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChangeOutcome {
    Removed,
    Narrowed,
    MustAsk,
    OnHost,
}

/// The policies over `agent` and what they change from what its own files
/// ask for
pub fn view(agent: &Agent, skill: &SkillConfig, policies: &PolicySet) -> AgentPolicies {
    match evaluate(agent, skill, policies) {
        Err(blocked) => AgentPolicies {
            blocked: Some(blocked),
            ..AgentPolicies::default()
        },
        Ok(effective) => AgentPolicies {
            policies: effective.policies,
            blocked: None,
            changes: effective
                .changes
                .into_iter()
                .map(|change| {
                    let (outcome, detail) = match change.outcome {
                        Outcome::Removed => (ChangeOutcome::Removed, None),
                        Outcome::Narrowed(left) => (ChangeOutcome::Narrowed, Some(left)),
                        Outcome::MustAsk => (ChangeOutcome::MustAsk, None),
                        Outcome::OnHost => (ChangeOutcome::OnHost, None),
                    };
                    PolicyChange {
                        item: change.item,
                        outcome,
                        detail,
                        by: change.by,
                    }
                })
                .collect(),
        },
    }
}

/// Logs a call a policy refused to `decisions.log`, with the policy that
/// refused it. `item` is as in [`PolicyChange::item`] (`tool shell`,
/// `pilot browser`).
pub async fn audit_refused(
    store: &crate::services::repo_store::RepoStore,
    agent_id: &str,
    item: &str,
) {
    let by = agent_policies(store, agent_id)
        .ok()
        .and_then(|view| view.changes.into_iter().find(|c| c.item == item))
        .map(|change| change.by);
    let line = serde_json::json!({
        "ts": crate::db::now_ms(),
        "agent_id": agent_id,
        "tool": item,
        "outcome": "refused",
        "policy": by,
    });
    crate::audit::record(crate::audit::AuditLog::Decisions, &line).await;
}

/// [`view`] for agent `id` in `store`
pub fn agent_policies(store: &RepoStore, id: &str) -> Result<AgentPolicies, String> {
    let (agent, skill) = store.declared_skill(id)?;
    Ok(view(&agent, &skill, &store.policies()?))
}

/// Every policy in `.nandi/policies/`, broken ones included, sorted by id
pub fn list(store: &RepoStore) -> Result<Vec<PolicySummary>, String> {
    store
        .repos()
        .policies
        .list()
        .map_err(|e| format!("Couldn't read the policies: {e}"))
}

/// What `.nandi/policies/<id>.md` holds after it changed on disk
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyFile {
    /// It parses, and applies from the next model call
    Valid,
    Removed,
    /// It doesn't parse (or read), so it blocks the agents it may cover
    Invalid(String),
}

/// Checks policy `id` as it is on disk now
pub fn check_file(store: &RepoStore, id: &str) -> PolicyFile {
    match store.repos().policies.read(id) {
        Ok(None) => PolicyFile::Removed,
        Ok(Some(text)) => match Policy::parse(id, &text) {
            Ok(_) => PolicyFile::Valid,
            Err(e) => PolicyFile::Invalid(e.to_string()),
        },
        Err(e) => PolicyFile::Invalid(e.to_string()),
    }
}

/// What the agent's skill.md asks for, as the policy evaluator reads it
fn request(agent: &Agent, skill: &SkillConfig) -> Request {
    let capabilities = agent.capabilities();
    Request {
        allowed_tools: skill.allowed_tools.clone(),
        approval: skill.requires_approval.clone(),
        network: capabilities.network().allow.clone(),
        aws: capabilities.aws().cloned(),
        model: Some(skill.model.clone()),
        folders: skill
            .folders
            .iter()
            .map(|folder| FolderRequest {
                id: folder.id.clone(),
                path: crate::services::connectors::expand_home(&folder.path)
                    .display()
                    .to_string(),
                mode: match folder.access {
                    Access::Read => FileMode::Read,
                    Access::Write => FileMode::Write,
                },
            })
            .collect(),
        connectors: skill.connectors.clone(),
        pilots: skill.pilots.clone(),
        home: dirs::home_dir().map(|home| home.display().to_string()),
    }
}

fn apply(mut skill: SkillConfig, effective: &Effective) -> SkillConfig {
    skill.allowed_tools = effective.allowed_tools.clone();
    skill.denied_tools = effective.denied_tools.keys().cloned().collect();
    skill.requires_approval = effective.approval.clone();
    skill.capabilities = Capabilities {
        aws: effective.aws.as_ref().map(aws),
        network: NetworkCapabilities {
            allow: effective
                .network
                .iter()
                .map(|rule| HostRule {
                    host: rule.host.clone(),
                    methods: rule.methods.clone(),
                })
                .collect(),
        },
    };
    skill.folders = skill
        .folders
        .into_iter()
        .filter_map(|folder| {
            let allowed = effective.folders.iter().find(|f| f.id == folder.id)?;
            Some(SharedFolder {
                access: match allowed.mode {
                    FileMode::Read => Access::Read,
                    FileMode::Write => folder.access,
                },
                ..folder
            })
        })
        .collect();
    skill.connectors = effective.connectors.clone();
    skill.connectors_on_host = effective.connectors_on_host.clone();
    skill.pilots = effective.pilots.clone();
    skill.browser_refusals = effective.browser_refusals.clone();
    skill.policies = effective.policies.clone();
    skill.model_rules = effective.model_rules.clone();
    skill.connector_forbid = effective.connector_forbid.clone();
    skill.connector_ask = effective.connector_ask.clone();
    skill
}

fn aws(policy: &domain_caps::AwsPolicy) -> AwsCapabilities {
    AwsCapabilities {
        mode: match policy.mode {
            domain_caps::AwsMode::Proxy => AwsMode::Proxy,
            domain_caps::AwsMode::Sts => AwsMode::Sts,
        },
        services: policy
            .services
            .iter()
            .map(|grant| AwsServiceGrant {
                service: grant.service.clone(),
                actions: grant.actions.clone(),
                resources: grant.resources.clone(),
            })
            .collect(),
        profile: policy.profile.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::{AgentId, ModelRef};
    use std::path::PathBuf;

    fn agent(frontmatter: &str) -> Agent {
        Agent::from_skill_md(
            AgentId::parse("writer").unwrap(),
            &format!("---\nname: Writer\n{frontmatter}---\n\nYou write.\n"),
        )
        .unwrap()
    }

    fn skill_for(agent: &Agent) -> SkillConfig {
        let default = ModelRef::parse("claude:claude-sonnet-5").unwrap();
        SkillConfig {
            folders: vec![
                SharedFolder {
                    id: "reports".into(),
                    path: "/data/reports".into(),
                    access: Access::Write,
                },
                SharedFolder {
                    id: "photos".into(),
                    path: "/media/photos".into(),
                    access: Access::Read,
                },
            ],
            ..SkillConfig::from_agent(agent, PathBuf::from("/agents/writer"), &default)
        }
    }

    fn set(policies: &[(&str, &str)]) -> PolicySet {
        PolicySet::parse(policies.iter().copied())
    }

    #[test]
    fn no_policies_leave_skill_md_as_it_is() {
        let agent = agent("network: { allow: [{ host: api.github.com }] }\n");
        let skill = skill_for(&agent);
        let narrowed = narrow(&agent, skill.clone(), &PolicySet::default()).unwrap();
        assert_eq!(narrowed, skill);
    }

    #[test]
    fn policies_narrow_tools_network_and_folders() {
        let agent =
            agent("network: { allow: [{ host: '*.github.com' }, { host: example.com }] }\n");
        let policies = set(&[(
            "safe",
            "---\napplies_to: [agent:writer]\ntools: { deny: [shell], ask: [read_file] }\n\
             network: { allow: [{ host: api.github.com }] }\nfiles: [{ path: /data, mode: read }]\n---\n",
        )]);
        let skill = narrow(&agent, skill_for(&agent), &policies).unwrap();
        assert!(!skill.allows_tool("shell"));
        assert!(skill.allows_tool("write_file"));
        assert!(skill.needs_approval("read_file"));
        let hosts: Vec<&str> = skill
            .capabilities
            .network
            .allow
            .iter()
            .map(|r| r.host.as_str())
            .collect();
        assert_eq!(hosts, ["api.github.com"]);
        assert_eq!(skill.folders.len(), 1, "photos isn't under /data");
        assert_eq!(skill.folders[0].access, Access::Read);
        assert_eq!(skill.policies, ["safe"]);
    }

    #[test]
    fn the_view_says_what_each_policy_changed_or_why_it_is_blocked() {
        let agent = agent("network: { allow: [{ host: example.com }] }\n");
        let policies = set(&[(
            "safe",
            "---\napplies_to: ['*']\ntools: { deny: [shell], ask: [read_file] }\n\
             network: { allow: [{ host: api.github.com }] }\n---\n",
        )]);
        let shown = view(&agent, &skill_for(&agent), &policies);
        assert_eq!(shown.policies, ["safe"]);
        assert!(shown.blocked.is_none());
        let outcome = |item: &str| {
            shown
                .changes
                .iter()
                .find(|c| c.item == item)
                .map(|c| (c.outcome, c.by.clone()))
        };
        assert_eq!(
            outcome("tool shell"),
            Some((ChangeOutcome::Removed, "safe".into()))
        );
        assert_eq!(
            outcome("tool read_file"),
            Some((ChangeOutcome::MustAsk, "safe".into()))
        );
        assert_eq!(
            outcome("host example.com"),
            Some((ChangeOutcome::Removed, "safe".into()))
        );

        let broken = set(&[("oops", "---\napplies_to: ['*']\ntools: { nope: 1 }\n---\n")]);
        let blocked = view(&agent, &skill_for(&agent), &broken);
        assert!(blocked.blocked.unwrap().contains("'oops'"));
        assert!(blocked.changes.is_empty());
    }

    #[test]
    fn tags_in_skill_md_select_policies() {
        let tagged = agent("tags: [finance]\n");
        let untagged = agent("");
        let policies = set(&[(
            "finance",
            "---\napplies_to: [tag:finance]\ntools: { deny: [shell] }\n---\n",
        )]);
        let skill = narrow(&tagged, skill_for(&tagged), &policies).unwrap();
        assert!(!skill.allows_tool("shell"));
        let skill = narrow(&untagged, skill_for(&untagged), &policies).unwrap();
        assert!(skill.allows_tool("shell"));
    }

    #[test]
    fn a_refused_model_or_a_broken_policy_stops_the_agent() {
        let agent = agent("");
        let only_gpt = set(&[(
            "gpt-only",
            "---\napplies_to: ['*']\nmodels: { allow: ['gpt:*'] }\n---\n",
        )]);
        let error = narrow(&agent, skill_for(&agent), &only_gpt).unwrap_err();
        assert!(error.contains("'gpt-only'"), "{error}");

        let broken = set(&[("oops", "---\napplies_to: ['*']\ntools: { nope: 1 }\n---\n")]);
        let error = narrow(&agent, skill_for(&agent), &broken).unwrap_err();
        assert!(error.contains("'oops'"), "{error}");
    }
}
