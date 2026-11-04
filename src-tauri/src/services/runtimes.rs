//! Agent runtimes: each agent's sandboxed container and the MCP session to
//! its tool server. Started on first use (or by `start_agent`), shared by
//! the orchestrator and the lifecycle commands.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use specta::Type;

use crate::llm::ToolSpec;
use crate::mcp_client::McpSession;
use crate::runtime::{self, AgentContainer, ProxyTokens};
use crate::services::agents::SkillConfig;
use crate::services::egress::EgressState;
use crate::services::events::{self, AgentsChanged, EventBus};

/// How often running containers are checked against Docker
const RECONCILE_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(15)
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ContainerStatus {
    /// No container (never started, or stopped)
    Idle,
    Starting,
    Running,
    /// The last start failed
    Error,
}

/// An agent's container as last seen by Nandi. A container stopped outside
/// Nandi is noticed by the reconciler (see [`AgentRuntimes::reconcile`]).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContainerState {
    pub status: ContainerStatus,
    /// Why the last start failed
    pub error: Option<String>,
}

impl ContainerState {
    const IDLE: Self = Self {
        status: ContainerStatus::Idle,
        error: None,
    };
}

pub struct ToolRuntime {
    pub container: AgentContainer,
    pub mcp: McpSession,
    /// Tools the container's MCP server offers
    pub tools: Vec<ToolSpec>,
}

/// Per-agent slot: an async lock so one agent's slow container start
/// doesn't block others, and concurrent callers don't start two.
type Slot = Arc<tokio::sync::Mutex<Option<Arc<ToolRuntime>>>>;

#[derive(Clone)]
pub struct AgentRuntimes {
    slots: Arc<Mutex<HashMap<String, Slot>>>,
    states: Arc<Mutex<HashMap<String, ContainerState>>>,
    proxy_tokens: ProxyTokens,
    egress: Arc<EgressState>,
    /// Announces status changes as `agent://agents_changed`
    bus: Option<EventBus>,
    /// Whether the background reconciler is running
    reconciling: Arc<AtomicBool>,
    /// Tests: every start fails with this error
    #[cfg(test)]
    fail_start: Option<String>,
}

impl AgentRuntimes {
    pub fn new(proxy_tokens: ProxyTokens, egress: Arc<EgressState>) -> Self {
        Self {
            slots: Arc::default(),
            states: Arc::default(),
            proxy_tokens,
            egress,
            bus: None,
            reconciling: Arc::default(),
            #[cfg(test)]
            fail_start: None,
        }
    }

    pub fn with_events(mut self, bus: EventBus) -> Self {
        self.bus = Some(bus);
        self
    }

    /// A runtime whose containers never start (no Docker needed).
    #[cfg(test)]
    pub fn failing(error: &str) -> Self {
        Self {
            fail_start: Some(error.to_string()),
            ..Self::new(
                ProxyTokens::default(),
                Arc::new(EgressState::new(
                    crate::services::secrets::Secrets::in_memory(),
                )),
            )
        }
    }

    pub fn state(&self, agent_id: &str) -> ContainerState {
        self.states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent_id)
            .cloned()
            .unwrap_or(ContainerState::IDLE)
    }

    fn set_state(&self, agent_id: &str, status: ContainerStatus, error: Option<String>) {
        let state = ContainerState { status, error };
        let changed = self
            .states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent_id.to_string(), state.clone())
            .is_none_or(|old| old != state);
        if status == ContainerStatus::Running {
            self.ensure_reconciler();
        }
        if let (true, Some(bus)) = (changed, &self.bus) {
            events::emit(
                bus,
                &AgentsChanged {
                    agent_id: agent_id.to_string(),
                },
            );
        }
    }

    /// Starts a runtime, tracking its status.
    async fn launch(&self, skill: &SkillConfig) -> Result<Arc<ToolRuntime>, String> {
        self.set_state(&skill.id, ContainerStatus::Starting, None);
        #[cfg(test)]
        if let Some(error) = &self.fail_start {
            self.set_state(&skill.id, ContainerStatus::Error, Some(error.clone()));
            return Err(error.clone());
        }
        match start(skill, &self.proxy_tokens, &self.egress).await {
            Ok(runtime) => {
                self.set_state(&skill.id, ContainerStatus::Running, None);
                Ok(Arc::new(runtime))
            }
            Err(e) => {
                self.set_state(&skill.id, ContainerStatus::Error, Some(e.clone()));
                Err(e)
            }
        }
    }

    fn slot(&self, agent_id: &str) -> Slot {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(agent_id.to_string())
            .or_default()
            .clone()
    }

    /// The running runtime, starting it if needed.
    pub async fn get_or_start(&self, skill: &SkillConfig) -> Result<Arc<ToolRuntime>, String> {
        let slot = self.slot(&skill.id);
        let mut current = slot.lock().await;
        if let Some(runtime) = current.as_ref() {
            return Ok(runtime.clone());
        }
        let runtime = self.launch(skill).await?;
        *current = Some(runtime.clone());
        Ok(runtime)
    }

    /// Replaces any running runtime with a fresh one.
    pub async fn restart(&self, skill: &SkillConfig) -> Result<Arc<ToolRuntime>, String> {
        let slot = self.slot(&skill.id);
        let mut current = slot.lock().await;
        if let Some(old) = current.take() {
            runtime::stop_container(&old.container.container_id).await;
        }
        let runtime = self.launch(skill).await?;
        *current = Some(runtime.clone());
        Ok(runtime)
    }

    /// Stops the agent's container and forgets its session.
    pub async fn stop(&self, agent_id: &str) {
        let slot = self.slot(agent_id);
        let old = slot.lock().await.take();
        self.teardown(agent_id, old).await;
    }

    async fn teardown(&self, agent_id: &str, old: Option<Arc<ToolRuntime>>) {
        if let Some(old) = old {
            runtime::stop_container(&old.container.container_id).await;
            // Close the session if no step is still using it
            if let Ok(old) = Arc::try_unwrap(old) {
                old.mcp.close().await;
            }
        }
        self.proxy_tokens.revoke(agent_id);
        self.set_state(agent_id, ContainerStatus::Idle, None);
    }

    /// Forgets a broken session (e.g. the container died) so the next
    /// call starts a fresh one.
    pub async fn invalidate(&self, agent_id: &str) {
        self.stop(agent_id).await;
    }

    /// Checks a `running` container against Docker. If it stopped outside
    /// Nandi (`docker stop`, a crash), forgets the session and marks it idle,
    /// so the UI updates and the next tool call starts a fresh one.
    pub async fn reconcile(&self, agent_id: &str) {
        if self.state(agent_id).status != ContainerStatus::Running {
            return;
        }
        let slot = self.slot(agent_id);
        // Busy: a start, restart, or stop is in progress and sets the state
        let Ok(mut current) = slot.try_lock() else {
            return;
        };
        let Some(container_id) = current
            .as_ref()
            .map(|runtime| runtime.container.container_id.clone())
        else {
            return;
        };
        match runtime::container_running(&container_id).await {
            Ok(true) => {}
            Ok(false) => {
                log::warn!("Agent '{agent_id}' container stopped outside Nandi");
                let old = current.take();
                drop(current);
                self.teardown(agent_id, old).await;
            }
            // Docker unreachable: keep the last known state
            Err(e) => log::debug!("Skipped container check for '{agent_id}': {e}"),
        }
    }

    fn running_agents(&self) -> Vec<String> {
        self.states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, state)| state.status == ContainerStatus::Running)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Starts the background reconciler unless it's already running. It
    /// polls every [`RECONCILE_INTERVAL`] and exits once no container is
    /// running, so an idle app does no polling.
    fn ensure_reconciler(&self) {
        if self.reconciling.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.reconciling.store(false, Ordering::SeqCst);
            return;
        };
        let runtimes = self.clone();
        handle.spawn(async move {
            loop {
                tokio::time::sleep(RECONCILE_INTERVAL).await;
                for agent_id in runtimes.running_agents() {
                    runtimes.reconcile(&agent_id).await;
                }
                if !runtimes.running_agents().is_empty() {
                    continue;
                }
                runtimes.reconciling.store(false, Ordering::SeqCst);
                // A container may have started since the check above; if so,
                // keep going unless that start already spawned a new loop
                if runtimes.running_agents().is_empty()
                    || runtimes.reconciling.swap(true, Ordering::SeqCst)
                {
                    break;
                }
            }
        });
    }
}

async fn start(
    skill: &SkillConfig,
    proxy_tokens: &ProxyTokens,
    egress: &EgressState,
) -> Result<ToolRuntime, String> {
    let proxy_token = proxy_tokens.issue(&skill.id);
    let container = match runtime::start_container(skill, &proxy_token, egress).await {
        Ok(container) => container,
        Err(e) => {
            proxy_tokens.revoke(&skill.id);
            return Err(e);
        }
    };
    let connected = async {
        let call_timeout = crate::mcp_client::call_timeout(skill.shell_timeout_s);
        let mcp = McpSession::connect(&container.container_id, call_timeout).await?;
        let tools = mcp.list_tools().await?;
        Ok::<_, String>((mcp, tools))
    }
    .await;
    match connected {
        Ok((mcp, tools)) => {
            log::info!(
                "Agent '{}' runtime ready with {} tools",
                skill.id,
                tools.len()
            );
            Ok(ToolRuntime {
                container,
                mcp,
                tools,
            })
        }
        Err(e) => {
            runtime::stop_container(&container.container_id).await;
            proxy_tokens.revoke(&skill.id);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for(condition: impl Fn() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition not met");
    }

    #[tokio::test]
    async fn reconciler_runs_only_while_a_container_is_running() {
        let runtimes = AgentRuntimes::failing("no docker");
        let reconciling = || runtimes.reconciling.load(Ordering::SeqCst);
        assert!(!reconciling());

        runtimes.set_state("a", ContainerStatus::Running, None);
        runtimes.set_state("b", ContainerStatus::Running, None);
        assert!(reconciling());

        runtimes.set_state("a", ContainerStatus::Idle, None);
        tokio::time::sleep(RECONCILE_INTERVAL * 3).await;
        assert!(reconciling(), "b is still running");

        runtimes.set_state("b", ContainerStatus::Idle, None);
        wait_for(|| !reconciling()).await;

        runtimes.set_state("a", ContainerStatus::Running, None);
        assert!(reconciling());
        runtimes.set_state("a", ContainerStatus::Idle, None);
        wait_for(|| !reconciling()).await;
    }

    #[tokio::test]
    async fn reconcile_leaves_agents_without_a_session_alone() {
        // No session to check (e.g. mid-start): nothing to reconcile
        let runtimes = AgentRuntimes::failing("no docker");
        runtimes.set_state("a", ContainerStatus::Running, None);
        runtimes.reconcile("a").await;
        assert_eq!(runtimes.state("a").status, ContainerStatus::Running);
        runtimes.reconcile("idle").await;
        assert_eq!(runtimes.state("idle"), ContainerState::IDLE);
        runtimes.set_state("a", ContainerStatus::Idle, None);
    }
}
