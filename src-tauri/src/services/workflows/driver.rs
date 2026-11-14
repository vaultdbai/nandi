//! Driving a run: one task per active run, starting each step and waiting
//! for its end.

use super::*;

/// Removes a run from `driving` when its driver stops.
struct DriveGuard<'a>(&'a WorkflowEngine, String);

impl Drop for DriveGuard<'_> {
    fn drop(&mut self) {
        self.0
            .driving
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.1);
    }
}

impl WorkflowEngine {
    pub(super) fn spawn_driver(self: &Arc<Self>, run_id: String) {
        let engine = Arc::clone(self);
        self.runtime.spawn(async move {
            if let Err(e) = engine.drive(&run_id).await {
                log::error!("Workflow run {run_id} stopped: {e}");
            }
        });
    }

    /// Advances a run from its persisted state until it finishes or pauses.
    async fn drive(&self, run_id: &str) -> Result<(), String> {
        if !self
            .driving
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string())
        {
            return Ok(()); // already being driven
        }
        let _guard = DriveGuard(self, run_id.to_string());

        loop {
            let id = run_id.to_string();
            let mut run = self
                .db
                .call(move |conn| get_run(conn, &id))
                .await?
                .ok_or_else(|| format!("Workflow run {run_id} not found"))?;
            if run.status != WorkflowRunStatus::Running {
                return Ok(());
            }
            // Done, or failed with `on_failure: continue`: settled
            let Some(index) = run.steps.iter().position(|s| {
                !(s.status == WorkflowStepStatus::Done
                    || (s.status == WorkflowStepStatus::Failed && s.continue_on_failure))
            }) else {
                run.status = WorkflowRunStatus::Completed;
                run.ended_at = Some(db::ts(db::now_ms()));
                self.save(&run).await?;
                log::info!("Workflow run {run_id} completed");
                events::emit(
                    &self.bus,
                    &WorkflowDone {
                        workflow_id: run.workflow_id,
                        run_id: run.id,
                    },
                );
                return Ok(());
            };

            match run.steps[index].status {
                WorkflowStepStatus::Pending => {
                    if !self.start_step(&mut run, index).await? {
                        return Ok(()); // paused or failed
                    }
                }
                WorkflowStepStatus::Running => {
                    if !self.finish_step(&mut run, index).await? {
                        return Ok(());
                    }
                }
                // Paused, or the run is already over
                _ => return Ok(()),
            }
        }
    }

    /// Sends a pending step's message (or pauses it for approval). Returns
    /// whether the run should keep going.
    async fn start_step(&self, run: &mut WorkflowRun, index: usize) -> Result<bool, String> {
        let outputs: HashMap<&str, &str> = run.steps[..index]
            .iter()
            .map(|s| (s.step_id.as_str(), s.output.as_deref().unwrap_or_default()))
            .collect();
        let input = render_input(&run.steps[index].template, &outputs);
        let instructions = run.steps[index]
            .instructions
            .as_deref()
            .map(|text| render_input(text, &outputs));
        if run.steps[index].ask.is_some() {
            let asked = run.steps[index]
                .ask
                .as_ref()
                .map(|ask| render_input(&ask.input, &outputs))
                .unwrap_or_default();
            return self.start_ask(run, index, asked).await;
        }
        let step = &mut run.steps[index];
        step.input = Some(input.clone());

        if step.manual {
            // Suspended until `complete_manual_step`; the state is all in
            // `workflow_runs`, so it survives restarts
            let now = db::now_ms();
            step.instructions = instructions;
            step.status = WorkflowStepStatus::AwaitingInput;
            step.started_at = Some(db::ts(now));
            step.deadline = step
                .deadline_hours
                .map(|hours| db::ts(now + i64::from(hours) * 3_600_000));
            let config = manual_steps::ManualStepConfig {
                step_id: step.step_id.clone(),
                instructions: step.instructions.clone().unwrap_or_default(),
                deadline: step.deadline.map(|d| d as i64),
            };
            let pending =
                manual_steps::PendingManualStep::new(&run.id, &run.workflow_id, config, now);
            run.status = WorkflowRunStatus::Waiting;
            self.save(run).await?;
            let pending = manual_steps::record_ready(&self.db, pending).await?;
            log::info!(
                "Workflow run {}: waiting on manual step '{}'",
                run.id,
                pending.step_id
            );
            events::emit(&self.bus, &pending);
            return Ok(false);
        }

        if step.requires_approval && !step.approved {
            step.status = WorkflowStepStatus::AwaitingApproval;
            let event = WorkflowStepAwaitingApproval {
                workflow_id: run.workflow_id.clone(),
                run_id: run.id.clone(),
                step_id: step.step_id.clone(),
                agent_id: step.agent_id.clone(),
                input,
            };
            run.status = WorkflowRunStatus::AwaitingApproval;
            self.save(run).await?;
            events::emit(&self.bus, &event);
            return Ok(false);
        }

        let agent_id = step.agent_id.clone();
        let critical = step.critical;
        let (id, workflow_id) = (agent_id.clone(), run.workflow_id.clone());
        let sent = match self
            .db
            .call(move |conn| {
                let conversation_id = db::ensure_agent_conversation(conn, &id)?;
                let name = workflow_name(conn, &workflow_id)?.unwrap_or(workflow_id);
                Ok((conversation_id, name))
            })
            .await
        {
            Ok((conversation_id, name)) => {
                let label = format!("{name} · step {}", index + 1);
                chat::submit(
                    &self.orchestrators,
                    &agent_id,
                    conversation_id,
                    input.clone(),
                    vec![],
                    None,
                    RunSource::workflow(&run.id, label, critical),
                )
                .await
            }
            Err(e) => Err(e),
        };
        let step = &mut run.steps[index];
        step.started_at = Some(db::ts(db::now_ms()));
        let sent = match sent {
            Ok(sent) => sent,
            Err(e) => {
                step.status = WorkflowStepStatus::Failed;
                step.ended_at = step.started_at;
                let error = format!("Step '{}' couldn't message '{agent_id}': {e}", step.step_id);
                step.error = Some(e);
                let step_id = step.step_id.clone();
                self.fail(run, Some(step_id), error).await?;
                return Ok(false);
            }
        };
        step.status = WorkflowStepStatus::Running;
        step.agent_run_id = Some(sent.run_id.clone());
        let event = WorkflowStepStarted {
            workflow_id: run.workflow_id.clone(),
            run_id: run.id.clone(),
            step_id: step.step_id.clone(),
            agent_id,
            agent_run_id: sent.run_id,
            input,
        };
        self.save(run).await?;
        events::emit(&self.bus, &event);
        Ok(true)
    }

    /// Waits for a running step's agent run to end and records its reply.
    /// Returns whether the run should keep going.
    async fn finish_step(&self, run: &mut WorkflowRun, index: usize) -> Result<bool, String> {
        let agent_run = match run.steps[index].agent_run_id.clone() {
            Some(agent_run_id) => self.wait_for_agent_run(&agent_run_id).await,
            None => Err("The step has no agent run".to_string()),
        };
        let step = &mut run.steps[index];
        step.ended_at = Some(db::ts(db::now_ms()));
        let error = match agent_run {
            Ok((agent_run, output)) if agent_run.status == RunStatus::Completed => {
                step.status = WorkflowStepStatus::Done;
                step.output = Some(output.clone());
                let event = WorkflowStepDone {
                    workflow_id: run.workflow_id.clone(),
                    run_id: run.id.clone(),
                    step_id: step.step_id.clone(),
                    output,
                };
                self.save(run).await?;
                events::emit(&self.bus, &event);
                return Ok(true);
            }
            Ok((agent_run, _)) => agent_run
                .summary
                .unwrap_or_else(|| format!("The agent run ended as {}", agent_run.status.as_str())),
            Err(e) => e,
        };
        step.status = WorkflowStepStatus::Failed;
        step.error = Some(error.clone());
        let step_id = step.step_id.clone();
        if step.continue_on_failure {
            log::warn!(
                "Workflow run {}: step '{step_id}' failed, continuing: {error}",
                run.id
            );
            self.save(run).await?;
            return Ok(true);
        }
        let error = format!("Step '{step_id}' failed: {error}");
        self.fail(run, Some(step_id), error).await?;
        Ok(false)
    }

    /// Waits for an agent run to settle (the newest run it compacted into
    /// has finished), returning that run and its final reply
    async fn wait_for_agent_run(&self, agent_run_id: &str) -> Result<(RunSummary, String), String> {
        RunWaiter::new(
            self.db.clone(),
            self.broadcast.subscribe(),
            RECHECK_INTERVAL,
        )
        .until_settled(agent_run_id)
        .await
    }

    pub(super) async fn fail(
        &self,
        run: &mut WorkflowRun,
        step_id: Option<String>,
        error: String,
    ) -> Result<(), String> {
        log::warn!("Workflow run {} failed: {error}", run.id);
        run.status = WorkflowRunStatus::Failed;
        run.ended_at = Some(db::ts(db::now_ms()));
        run.error = Some(error.clone());
        self.save(run).await?;
        events::emit(
            &self.bus,
            &WorkflowFailed {
                workflow_id: run.workflow_id.clone(),
                run_id: run.id.clone(),
                step_id,
                error,
            },
        );
        Ok(())
    }
}
