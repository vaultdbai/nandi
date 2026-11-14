//! One agent's orchestrator: its command loop, its queue and the turn in flight.

use super::*;

pub(super) struct InFlight {
    pub(super) run_id: String,
    pub(super) abort: AbortHandle,
}

/// The child task executing an agent's run. Its supervisor holds it too,
/// outside the actor, so after a panic the supervisor can still abort the
/// orphan child and fail its run (a restarted orchestrator never knew it).
#[derive(Clone, Default)]
pub(super) struct InFlightSlot(Arc<Mutex<Option<InFlight>>>);

impl InFlightSlot {
    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Option<InFlight>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(super) struct Orchestrator {
    pub(super) agent_id: String,
    pub(super) rx: mpsc::Receiver<OrchestratorCommand>,
    /// For the child task to report back
    pub(super) tx: mpsc::Sender<OrchestratorCommand>,
    pub(super) bus: EventBus,
    pub(super) deps: RunDeps,
    /// The child task executing steps, while one is
    pub(super) in_flight: InFlightSlot,
    /// The agent's one active turn (running or awaiting approval). While
    /// it's set, new requests queue: this flag, not the channel, is what
    /// keeps a second turn from starting.
    pub(super) current: Option<String>,
    /// A2A: for the `delegate_to_agent` tool
    pub(super) delegation: DelegationSlot,
}

impl Orchestrator {
    pub(super) async fn run(mut self) {
        log::info!("Orchestrator for '{}' started", self.agent_id);
        let name = display_name(&self.deps.store, &self.agent_id);
        match self.bus.reannounce_approvals(&self.agent_id, &name).await {
            Ok(0) => {}
            Ok(n) => log::info!("'{}' has {n} pending approvals from before", self.agent_id),
            Err(e) => log::warn!(
                "Failed to re-announce approvals for '{}': {e}",
                self.agent_id
            ),
        }

        // Pick up where the last process left off: an approval-blocked run
        // is still the current turn, and queued requests wait behind it
        let agent_id = self.agent_id.clone();
        match self
            .bus
            .db
            .call(move |conn| db::active_agent_run(conn, &agent_id))
            .await
        {
            Ok(active) => self.current = active.map(|run| run.id),
            Err(e) => log::error!(
                "Failed to load the current turn of '{}': {e}",
                self.agent_id
            ),
        }
        self.start_next().await;

        while let Some(command) = self.rx.recv().await {
            match command {
                OrchestratorCommand::SendMessage { request, reply } => {
                    let _ = reply.send(self.accept(*request).await);
                }
                OrchestratorCommand::Approve {
                    approval_id,
                    decision,
                } => {
                    let status = match decision {
                        ApprovalDecision::Approved => ApprovalStatus::Approved,
                        ApprovalDecision::Rejected => ApprovalStatus::Rejected,
                    };
                    if let Err(e) = self.close_approval(&approval_id, status).await {
                        log::warn!("Approve {approval_id} for '{}' failed: {e}", self.agent_id);
                    }
                }
                OrchestratorCommand::ExpireApproval { approval_id } => {
                    match self
                        .close_approval(&approval_id, ApprovalStatus::Expired)
                        .await
                    {
                        Ok(()) => {
                            log::info!("Approval {approval_id} for '{}' expired", self.agent_id)
                        }
                        Err(e) => {
                            log::warn!("Expiring {approval_id} for '{}' failed: {e}", self.agent_id)
                        }
                    }
                }
                OrchestratorCommand::Cancel { run_id } => {
                    if let Err(e) = self.cancel(&run_id).await {
                        log::warn!("Cancel {run_id} for '{}' failed: {e}", self.agent_id);
                    }
                }
                OrchestratorCommand::CancelQueued { run_id, reply } => {
                    let _ = reply.send(self.cancel_queued(&run_id).await);
                }
                OrchestratorCommand::Reprioritize {
                    run_id,
                    priority,
                    reply,
                } => {
                    let _ = reply.send(self.reprioritize(&run_id, priority).await);
                }
                OrchestratorCommand::StepFinished { run_id, outcome } => {
                    self.step_finished(&run_id, outcome).await;
                }
                OrchestratorCommand::Shutdown => break,
                #[cfg(test)]
                OrchestratorCommand::Crash => panic!("the orchestrator crashed (test)"),
            }
        }

        if let Some(in_flight) = self.in_flight.lock().take() {
            in_flight.abort.abort();
        }
        log::info!("Orchestrator for '{}' stopped", self.agent_id);
    }

    async fn context(&self, run_id: &str) -> Result<RunContext, String> {
        let id = run_id.to_string();
        let run = self
            .bus
            .db
            .call(move |conn| db::get_run(conn, &id))
            .await?
            .ok_or_else(|| format!("Run {run_id} not found"))?;
        Ok(RunContext {
            agent_id: self.agent_id.clone(),
            agent_name: display_name(&self.deps.store, &self.agent_id),
            conversation_id: run.conversation_id,
            run_id: run.id,
        })
    }

    /// Persists a turn request: started now if the agent is idle and
    /// nothing is waiting, else queued. Returns whether it was queued.
    async fn accept(&mut self, request: TurnRequest) -> Result<bool, String> {
        let TurnRequest {
            run_id,
            conversation_id,
            message_id,
            text,
            input,
            attachments,
            source,
        } = request;
        let agent_id = self.agent_id.clone();
        let queue_empty = self
            .bus
            .db
            .call(move |conn| db::queue_is_empty(conn, &agent_id))
            .await?;
        let start_now = self.current.is_none() && queue_empty;

        let now = db::now_ms();
        let run = RunSummary {
            id: run_id.clone(),
            agent_id: self.agent_id.clone(),
            conversation_id: conversation_id.clone(),
            status: if start_now {
                RunStatus::Running
            } else {
                RunStatus::Queued
            },
            input,
            summary: None,
            started_at: db::ts(now),
            ended_at: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
            model: None,
            source_type: source.source_type,
            source_id: source.source_id.clone(),
            source_label: source.source_label.clone(),
            priority: source.priority,
            queued_at: (!start_now).then(|| db::ts(now)),
            parent_run_id: None,
            compaction_summary: None,
        };

        if start_now {
            let message = MessageRow::new(
                &message_id,
                &conversation_id,
                Some(&run_id),
                role::USER,
                text,
                UserMeta { attachments },
            );
            let ctx = RunContext {
                agent_id: self.agent_id.clone(),
                agent_name: display_name(&self.deps.store, &self.agent_id),
                conversation_id,
                run_id: run_id.clone(),
            };
            if !self.bus.start_run(ctx, run, message).await? {
                return Err(
                    "This conversation is still working on the previous message".to_string()
                );
            }
            self.current = Some(run_id.clone());
            self.start_executor(run_id);
            return Ok(false);
        }

        let entry = QueueRow {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: self.agent_id.clone(),
            run_id: run_id.clone(),
            source_type: source.source_type,
            source_id: source.source_id,
            source_label: source.source_label,
            priority: source.priority,
            message: text,
            message_id,
            attachments,
            created_at: now,
        };
        self.bus.enqueue_run(run, entry).await?;
        log::info!("Queued run {run_id} for '{}'", self.agent_id);
        // Idle with requests already waiting (e.g. after a failed start)
        self.start_next().await;
        Ok(self.current.as_deref() != Some(run_id.as_str()))
    }

    /// While the agent is idle, starts its next queued request.
    async fn start_next(&mut self) {
        if self.current.is_some() {
            return;
        }
        let name = display_name(&self.deps.store, &self.agent_id);
        match self.bus.dequeue_next(&self.agent_id, &name).await {
            Ok(Some(run)) => {
                log::info!("Starting queued run {} for '{}'", run.id, self.agent_id);
                self.current = Some(run.id.clone());
                self.start_executor(run.id);
            }
            Ok(None) => {}
            Err(e) => log::error!("Failed to start the next run of '{}': {e}", self.agent_id),
        }
    }

    /// The current turn ended: the agent is free for the next request.
    async fn turn_ended(&mut self, run_id: &str) {
        if self.current.as_deref() == Some(run_id) {
            self.current = None;
        }
        self.start_next().await;
    }

    /// Runs the run's steps in a child task until it completes or suspends.
    fn start_executor(&mut self, run_id: String) {
        let mut slot = self.in_flight.lock();
        if let Some(in_flight) = slot.as_ref() {
            log::warn!(
                "'{}' is already executing {}; ignoring {run_id}",
                self.agent_id,
                in_flight.run_id
            );
            return;
        }

        let (deps, agent_id, tx) = (self.deps.clone(), self.agent_id.clone(), self.tx.clone());
        let delegation = self.delegation.clone();
        let id = run_id.clone();
        let child =
            tokio::spawn(async move { execute_run(&deps, &delegation, &agent_id, &id).await });
        let abort = child.abort_handle();

        let id = run_id.clone();
        tokio::spawn(async move {
            let outcome = match child.await {
                Ok(Ok(RunEnd::Done)) => StepOutcome::Ok,
                Ok(Ok(RunEnd::ContinuedIn(new_run_id))) => StepOutcome::Continued(new_run_id),
                Ok(Err(e)) => StepOutcome::Failed(e),
                Err(e) if e.is_panic() => StepOutcome::Panicked(panic_message(e.into_panic())),
                // Aborted by Cancel/Shutdown, which handle the run themselves
                Err(_) => return,
            };
            let _ = tx
                .send(OrchestratorCommand::StepFinished {
                    run_id: id,
                    outcome,
                })
                .await;
        });

        *slot = Some(InFlight { run_id, abort });
    }

    pub(super) async fn step_finished(&mut self, run_id: &str, outcome: StepOutcome) {
        self.in_flight.lock().take_if(|f| f.run_id == run_id);
        let error = match outcome {
            StepOutcome::Ok => {
                // Completed, or suspended for approval (still current)
                let id = run_id.to_string();
                let status = self
                    .bus
                    .db
                    .call(move |conn| db::get_run(conn, &id))
                    .await
                    .ok()
                    .flatten()
                    .map(|run| run.status);
                match status {
                    None => self.turn_ended(run_id).await,
                    Some(status) if status.is_finished() => self.turn_ended(run_id).await,
                    Some(RunStatus::AwaitingApproval) => {}
                    // Running again: its approval was answered while this
                    // step was still finishing, so `close_approval` found
                    // the executor busy and couldn't resume it. It resumes
                    // now.
                    Some(_) if self.current.as_deref() == Some(run_id) => {
                        self.start_executor(run_id.to_string());
                    }
                    Some(_) => {}
                }
                return;
            }
            StepOutcome::Continued(new_run_id) => {
                // The child run is the same turn: it stays current (so the
                // queue keeps waiting, and Approve/Cancel target it)
                log::info!(
                    "Run {run_id} of '{}' continues in {new_run_id}",
                    self.agent_id
                );
                if self.current.as_deref() == Some(run_id) {
                    self.current = Some(new_run_id.clone());
                }
                self.start_executor(new_run_id);
                return;
            }
            StepOutcome::Failed(e) => e,
            StepOutcome::Panicked(e) => {
                log::error!("Run {run_id} of '{}' panicked: {e}", self.agent_id);
                let agent_id = self.agent_id.clone();
                let message = format!("Run panicked: {e}");
                let _ = self
                    .bus
                    .db
                    .call(move |conn| db::set_agent_state(conn, &agent_id, "error", Some(&message)))
                    .await;
                INTERNAL_ERROR.to_string()
            }
        };
        log::warn!("Run {run_id} of '{}' failed: {error}", self.agent_id);
        match self.context(run_id).await {
            Ok(ctx) => {
                if let Err(e) = self
                    .bus
                    .turn_done(&ctx, RunStatus::Failed, Some(error))
                    .await
                {
                    log::error!("Failed to record failure of {run_id}: {e}");
                }
            }
            Err(e) => log::error!("{e}"),
        }
        self.turn_ended(run_id).await;
    }

    pub(super) async fn close_approval(
        &mut self,
        approval_id: &str,
        status: ApprovalStatus,
    ) -> Result<(), String> {
        let id = approval_id.to_string();
        let approval = self
            .bus
            .db
            .call(move |conn| db::get_approval(conn, &id))
            .await?
            .ok_or("Approval not found")?;
        if approval.agent_id != self.agent_id || approval.status != ApprovalStatus::Pending {
            return Err("Approval is not pending for this agent".to_string());
        }
        if self.current.as_deref() != Some(approval.run_id.as_str()) {
            return Err("Approval is not for this agent's current turn".to_string());
        }
        let ctx = self.context(&approval.run_id).await?;
        let run_id = approval.run_id.clone();
        self.bus.approval_closed(&ctx, approval, status).await?;
        self.start_executor(run_id);
        Ok(())
    }

    /// Stops the current turn. Queued runs aren't active, so they're
    /// left alone (see `cancel_queued`).
    async fn cancel(&mut self, run_id: &str) -> Result<(), String> {
        let id = run_id.to_string();
        let run = self.bus.db.call(move |conn| db::get_run(conn, &id)).await?;
        if !run.is_some_and(|r| r.agent_id == self.agent_id && r.status.is_active()) {
            return Ok(());
        }
        let in_flight = self.in_flight.lock().take_if(|f| f.run_id == run_id);
        if let Some(in_flight) = in_flight {
            in_flight.abort.abort();
        }
        let ctx = self.context(run_id).await?;
        self.bus.turn_done(&ctx, RunStatus::Cancelled, None).await?;
        log::info!("Cancelled run {run_id} of '{}'", self.agent_id);
        self.turn_ended(run_id).await;
        Ok(())
    }

    async fn cancel_queued(&mut self, run_id: &str) -> Result<(), String> {
        if self.current.as_deref() == Some(run_id) {
            return Err("This run has already started; stop it instead".to_string());
        }
        if !self
            .bus
            .cancel_queued(&display_name(&self.deps.store, &self.agent_id), run_id)
            .await?
        {
            return Err("This run is no longer queued".to_string());
        }
        log::info!("Removed run {run_id} from the queue of '{}'", self.agent_id);
        Ok(())
    }

    async fn reprioritize(&mut self, run_id: &str, priority: i32) -> Result<(), String> {
        if !self.bus.reprioritize(run_id, priority).await? {
            return Err("This run is no longer queued".to_string());
        }
        Ok(())
    }
}
