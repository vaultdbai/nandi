//! Every agent's orchestrator, started on demand and restarted after a panic.

use super::*;

/// The default model (Studio's model, and agents' without one): the
/// environment's or the repo's `.nandi/llm.yaml`, else settings (see
/// `config::repo_config`). `llm::stream` swaps in a provider the user has
/// a key for when this one has none.
pub(crate) fn default_model(store: &RepoStore) -> String {
    crate::services::config::repo_config::default_model(store)
}

pub(crate) fn display_name(store: &RepoStore, agent_id: &str) -> String {
    if agent_id == STUDIO_AGENT_ID {
        "Nandi".to_string()
    } else {
        store.display_name(agent_id)
    }
}

// ---------------------------------------------------------------------------
// Registry (managed state)
// ---------------------------------------------------------------------------

struct Registry {
    senders: Mutex<HashMap<String, mpsc::Sender<OrchestratorCommand>>>,
    bus: EventBus,
    deps: RunDeps,
    /// A2A: shared with every orchestrator
    delegation: DelegationSlot,
    /// Where tasks are spawned: Tauri's runtime on desktop, our own when
    /// headless. Explicit so this works outside a Tauri app.
    runtime: tokio::runtime::Handle,
}

/// All orchestrator tasks, keyed by agent ID.
#[derive(Clone)]
pub struct Orchestrators(Arc<Registry>);

impl Orchestrators {
    pub fn new(deps: RunDeps, runtime: tokio::runtime::Handle) -> Self {
        Self(Arc::new(Registry {
            senders: Mutex::default(),
            bus: deps.bus.clone(),
            deps,
            delegation: DelegationSlot::default(),
            runtime,
        }))
    }

    /// A2A: the delegator `delegate_to_agent` calls
    pub fn set_delegator(&self, delegator: &Arc<dyn A2ADelegator>) {
        self.0.delegation.set(delegator);
    }

    /// The delegator, once delegation is set up (workflow ask steps)
    pub fn delegator(&self) -> Option<Arc<dyn A2ADelegator>> {
        self.0.delegation.get()
    }

    /// Clarification: the service `ask_requester` calls
    pub fn set_clarifier(&self, service: &Arc<a2a::clarification::ClarificationService>) {
        self.0.delegation.set_clarifier(service);
    }

    /// Clarification: the service `ask_requester` calls, once set
    pub fn clarifier(&self) -> Option<Arc<a2a::clarification::ClarificationService>> {
        self.0.delegation.clarifier()
    }

    pub fn bus(&self) -> &EventBus {
        &self.0.bus
    }

    /// What runs need (for the turn middlewares' service APIs)
    pub fn deps(&self) -> &RunDeps {
        &self.0.deps
    }

    /// The gold copy the agents are defined in.
    pub fn store(&self) -> &RepoStore {
        &self.0.deps.store
    }

    /// The agent's command channel, spawning its orchestrator if needed
    /// (e.g. for an agent folder created after startup).
    pub fn ensure(&self, agent_id: &str) -> mpsc::Sender<OrchestratorCommand> {
        let mut senders = self.0.senders.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = senders.get(agent_id).filter(|tx| !tx.is_closed()) {
            return tx.clone();
        }
        let tx = self.spawn(agent_id, 0, InFlightSlot::default());
        senders.insert(agent_id.to_string(), tx.clone());
        tx
    }

    /// Spawns the task under a supervisor that logs panics, marks the agent
    /// as errored, stops the run the crash orphaned, and restarts it (a
    /// bounded number of times). The slot outlives each orchestrator.
    fn spawn(
        &self,
        agent_id: &str,
        restarts: u32,
        in_flight: InFlightSlot,
    ) -> mpsc::Sender<OrchestratorCommand> {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let orchestrator = Orchestrator {
            agent_id: agent_id.to_string(),
            rx,
            tx: tx.clone(),
            bus: self.0.bus.clone(),
            deps: self.0.deps.clone(),
            in_flight: in_flight.clone(),
            current: None,
            delegation: self.0.delegation.clone(),
        };
        let (registry, agent_id) = (self.clone(), agent_id.to_string());

        self.0.runtime.spawn(async move {
            let Err(e) = tokio::spawn(orchestrator.run()).await else {
                return; // clean shutdown
            };
            let message = if e.is_panic() {
                panic_message(e.into_panic())
            } else {
                e.to_string()
            };
            log::error!("Orchestrator for '{agent_id}' crashed: {message}");
            let (id, error) = (agent_id.clone(), format!("Orchestrator crashed: {message}"));
            let _ = registry
                .0
                .bus
                .db
                .call(move |conn| db::set_agent_state(conn, &id, "error", Some(&error)))
                .await;
            let orphan = in_flight.lock().take();
            if let Some(orphan) = orphan {
                orphan.abort.abort();
                registry.fail_orphan(&agent_id, &orphan.run_id).await;
            }

            if restarts >= MAX_RESTARTS {
                log::error!("Not restarting '{agent_id}' after {restarts} crashes");
                return;
            }
            tokio::time::sleep(RESTART_DELAY).await;
            let tx = registry.spawn(&agent_id, restarts + 1, in_flight);
            registry
                .0
                .senders
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(agent_id, tx);
        });
        tx
    }

    /// Fails the run a crashed orchestrator's child was executing, so the
    /// restarted one doesn't wait on it as the current turn. A run that
    /// already completed or paused for approval is left as it is.
    async fn fail_orphan(&self, agent_id: &str, run_id: &str) {
        let id = run_id.to_string();
        let run = match self.0.bus.db.call(move |conn| db::get_run(conn, &id)).await {
            Ok(Some(run)) if run.status == RunStatus::Running => run,
            Ok(_) => return,
            Err(e) => {
                log::error!("Failed to load the orphaned run {run_id}: {e}");
                return;
            }
        };
        let ctx = RunContext {
            agent_id: agent_id.to_string(),
            agent_name: display_name(&self.0.deps.store, agent_id),
            conversation_id: run.conversation_id,
            run_id: run.id,
        };
        let error = Some(INTERNAL_ERROR.to_string());
        match self.0.bus.turn_done(&ctx, RunStatus::Failed, error).await {
            Ok(()) => log::warn!("Failed run {run_id} of '{agent_id}': its orchestrator crashed"),
            Err(e) => log::error!("Failed to record failure of {run_id}: {e}"),
        }
    }

    pub async fn send(&self, agent_id: &str, command: OrchestratorCommand) -> Result<(), String> {
        self.ensure(agent_id)
            .send(command)
            .await
            .map_err(|_| format!("The orchestrator for '{agent_id}' is not running"))
    }

    /// Sends a command built around a reply channel and awaits the reply.
    pub async fn request<T>(
        &self,
        agent_id: &str,
        command: impl FnOnce(oneshot::Sender<Result<T, String>>) -> OrchestratorCommand,
    ) -> Result<T, String> {
        let (reply, response) = oneshot::channel();
        self.send(agent_id, command(reply)).await?;
        response
            .await
            .map_err(|_| format!("The orchestrator for '{agent_id}' stopped"))?
    }

    /// Tests: whether the agent's orchestrator is up, without starting it
    #[cfg(test)]
    pub fn is_running(&self, agent_id: &str) -> bool {
        let senders = self.0.senders.lock().unwrap_or_else(|e| e.into_inner());
        senders.get(agent_id).is_some_and(|tx| !tx.is_closed())
    }

    /// Tests: crashes the agent's running orchestrator (`false`: none runs)
    #[cfg(test)]
    pub fn crash(&self, agent_id: &str) -> bool {
        let senders = self.0.senders.lock().unwrap_or_else(|e| e.into_inner());
        senders
            .get(agent_id)
            .is_some_and(|tx| tx.try_send(OrchestratorCommand::Crash).is_ok())
    }

    /// Stops one agent's orchestrator and forgets it (e.g. when deleted).
    pub fn remove(&self, agent_id: &str) {
        let tx = self
            .0
            .senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent_id);
        if let Some(tx) = tx {
            let _ = tx.try_send(OrchestratorCommand::Shutdown);
        }
    }

    /// Asks every orchestrator to stop. Called on app exit.
    pub fn shutdown_all(&self) {
        let senders: Vec<_> = self
            .0
            .senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        for (agent_id, tx) in senders {
            if tx.try_send(OrchestratorCommand::Shutdown).is_err() {
                log::warn!("Could not signal shutdown to '{agent_id}'");
            }
        }
    }
}
