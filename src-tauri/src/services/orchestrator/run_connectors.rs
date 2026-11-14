//! The runtime connectors an agent's run may call: their tools and calls.

use super::*;

/// The connector registry for runs.
pub(super) fn connector_registry(deps: &RunDeps) -> Option<&ConnectorRegistry> {
    deps.connectors.as_deref()
}

/// The connectors `agent_id` may use (`connectors: [...]`, narrowed by
/// its policies)
#[derive(Default)]
struct AgentConnectors {
    declared: Vec<String>,
    /// The ones a policy lets run on the host
    on_host: Vec<String>,
    /// Tools a policy denies it
    denied: Vec<String>,
}

fn declared_connectors(deps: &RunDeps, agent_id: &str) -> AgentConnectors {
    deps.store
        .skill(agent_id)
        .map(|skill| AgentConnectors {
            declared: skill.connectors,
            on_host: skill.connectors_on_host,
            denied: skill.denied_tools,
        })
        .unwrap_or_default()
}

/// Appends the tools of the connectors the agent declares (prefixed
/// `<connector_id>__`) to `tools`, which holds the container's tools.
/// Unhealthy connectors are left out and the UI gets `agent://warning`.
/// Returns the prefixed names that need approval.
pub(super) async fn merge_connector_tools(
    registry: Option<&ConnectorRegistry>,
    deps: &RunDeps,
    agent_id: &str,
    tools: &mut Vec<ToolSpec>,
) -> HashSet<String> {
    let Some(registry) = registry else {
        return HashSet::new();
    };
    let AgentConnectors {
        declared,
        on_host,
        denied,
    } = declared_connectors(deps, agent_id);
    if declared.is_empty() {
        return HashSet::new();
    }
    let manifest = deps.store.read(|repo| repo.connectors().clone());
    let taken: HashSet<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    let set = registry
        .agent_tools_placed(&deps.secrets, &manifest, &declared, &on_host, &taken)
        .await;
    for message in set.warnings {
        log::warn!("Agent '{agent_id}': {message}");
        events::emit(
            &deps.bus,
            &AgentWarning {
                agent_id: agent_id.to_string(),
                message,
            },
        );
    }
    for changed in &set.changed {
        events::emit(&deps.bus, changed);
    }
    tools.extend(
        set.tools
            .into_iter()
            .filter(|tool| !denied.contains(&tool.name)),
    );
    set.sensitive
}

/// Runs `call` on a connector if it names a tool of one the agent declares;
/// `None` sends it on to the container.
pub(super) async fn connector_call(
    registry: Option<&ConnectorRegistry>,
    deps: &RunDeps,
    ctx: &RunContext,
    call: &PlannedCall,
) -> Option<ToolOutcome> {
    let registry = registry?;
    let agent = deps.store.agent(&ctx.agent_id).ok();
    let connectors = declared_connectors(deps, &ctx.agent_id);
    let options = agent
        .as_ref()
        .map(connectors::declared_options)
        .unwrap_or_default();
    // Its sites as its policies leave them; none when it can't be read
    let allow = deps
        .store
        .skill(&ctx.agent_id)
        .map(|skill| skill.capabilities.network.allow)
        .unwrap_or_default();
    let options = connectors::with_allowed_sites(options, &connectors.declared, &allow);
    registry
        .call_agent_tool_placed(
            &ctx.agent_id,
            &connectors.declared,
            &options,
            &connectors.on_host,
            &call.tool,
            &call.args,
        )
        .await
}
