//! Ask steps: finding who to ask, sending them one request, and settling the
//! run when the answer comes back.

use super::*;

/// How often waiting ask steps are checked for their answer
const ASK_SETTLE_INTERVAL: Duration = Duration::from_secs(15);

/// Where an ask step's request goes
struct ResolvedAsk {
    /// A provider card ID; `None`: any provider of the capability
    target: Option<String>,
    capability: String,
    /// Who, in words
    who: String,
}

/// How many people an ask by topic considers
const ASK_FIND_LIMIT: u32 = 10;

/// `person`: that person's gateway, asked with `ask-me`. `find`: the
/// relay's best match for the topic (from what people publish) that takes
/// requests, through their gateway; without a connected relay,
/// `RelayUnavailable`. `capability`: any provider.
async fn resolve_ask(
    db: &Db,
    relay: Option<&Arc<dyn RelayClient>>,
    ask: &WorkflowAsk,
) -> Result<ResolvedAsk, String> {
    use crate::services::a2a::assignment::ASK_ME;
    use crate::services::domain::AgentCard;
    use crate::services::gateway::GATEWAY_AGENT_ID;
    use crate::services::network;
    let named = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    if let Some(capability) = named(&ask.capability) {
        return Ok(ResolvedAsk {
            target: None,
            who: format!("whoever offers '{capability}'"),
            capability,
        });
    }
    let members = network::known_members(db);
    if let Some(person) = named(&ask.person) {
        let who = members
            .iter()
            .find(|m| m.instance_id == person)
            .map(|m| m.display_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| person.clone());
        return Ok(ResolvedAsk {
            target: Some(AgentCard::card_id(&person, GATEWAY_AGENT_ID)),
            capability: ASK_ME.to_string(),
            who,
        });
    }
    let topic = named(&ask.find).ok_or("The ask step names nobody to ask")?;
    let relay = unavailable::connected(relay)?;
    let local = network::local_instance_id(db)?;
    relay
        .find_people(&topic, ASK_FIND_LIMIT)
        .await?
        .matches
        .into_iter()
        .find(|m| m.takes_requests && m.instance_id != local)
        .map(|m| ResolvedAsk {
            target: Some(AgentCard::card_id(&m.instance_id, GATEWAY_AGENT_ID)),
            capability: m.ask_capability,
            who: m.name,
        })
        .ok_or_else(|| format!("Nobody on your network lists '{topic}' as theirs"))
}

impl WorkflowEngine {
    /// An ask step (network v1, Part 5): finds who to ask, waits for the
    /// owner's approval when it found them by topic (or the step asks
    /// first), then sends one request and suspends the run until the answer
    /// comes back (`settle_asks`). Returns whether the run should keep going.
    pub(super) async fn start_ask(
        &self,
        run: &mut WorkflowRun,
        index: usize,
        asked: String,
    ) -> Result<bool, String> {
        let step = &mut run.steps[index];
        let Some(ask) = step.ask.clone() else {
            return Ok(true);
        };
        step.input = Some(asked.clone());
        if step.ask_capability.is_none() {
            match resolve_ask(&self.db, self.relay().as_ref(), &ask).await {
                Ok(resolved) => {
                    step.ask_target = resolved.target;
                    step.ask_capability = Some(resolved.capability);
                    step.asked = Some(resolved.who);
                }
                Err(e) => return self.ask_failed(run, index, e).await,
            }
        }
        let step = &mut run.steps[index];
        let who = step.asked.clone().unwrap_or_default();
        if (step.requires_approval || ask.find.is_some()) && !step.approved {
            step.status = WorkflowStepStatus::AwaitingApproval;
            let event = WorkflowStepAwaitingApproval {
                workflow_id: run.workflow_id.clone(),
                run_id: run.id.clone(),
                step_id: step.step_id.clone(),
                agent_id: step
                    .ask_target
                    .clone()
                    .or_else(|| step.ask_capability.clone())
                    .unwrap_or_default(),
                input: format!("Ask {who}: {asked}"),
            };
            run.status = WorkflowRunStatus::AwaitingApproval;
            self.save(run).await?;
            events::emit(&self.bus, &event);
            return Ok(false);
        }
        let Some(delegator) = self.orchestrators.delegator() else {
            return self
                .ask_failed(run, index, "Delegation is unavailable".to_string())
                .await;
        };
        let capability = step.ask_capability.clone().unwrap_or_default();
        let mut payload = serde_json::json!({ "text": asked });
        if let Some(due) = &ask.due {
            payload["due"] = Value::String(due.clone());
        }
        let now = db::now_ms();
        let task_id = uuid::Uuid::new_v4().to_string();
        step.task_id = Some(task_id.clone());
        step.status = WorkflowStepStatus::AwaitingInput;
        step.started_at = Some(db::ts(now));
        step.deadline = Some(db::ts(crate::services::a2a::assignment::request_due_at(
            &capability,
            &payload,
            now,
        )));
        let request = crate::services::a2a::DelegationRequest {
            task_id: Some(task_id.clone()),
            requester_agent: None,
            capability: capability.clone(),
            input: payload.clone(),
            target: step.ask_target.clone(),
        };
        run.status = WorkflowRunStatus::Waiting;
        self.save(run).await?;
        log::info!(
            "Workflow run {}: step '{}' asks {who} (task {task_id})",
            run.id,
            run.steps[index].step_id
        );
        // The answer may take days: it lands in the delegation history,
        // where `settle_asks` picks it up. A request that can't even be
        // sent is recorded there as failed, the same way.
        let db = self.db.clone();
        let target = request.target.clone();
        self.runtime.spawn(async move {
            if let Err(e) = delegator.delegate_request(request).await {
                let mut failed = crate::services::a2a::history::placeholder(
                    &task_id,
                    &capability,
                    &payload,
                    target.as_deref(),
                );
                crate::services::a2a::history::finish(
                    &mut failed,
                    crate::services::a2a::TaskStatus::Failed,
                    Some(e),
                );
                if let Err(e) = crate::services::a2a::history::insert(&db, &failed).await {
                    log::warn!("Ask task {task_id}: couldn't record the failure: {e}");
                }
            }
        });
        Ok(false)
    }

    /// An ask step that couldn't be sent: failed, and the run stops unless
    /// the step continues on failure
    async fn ask_failed(
        &self,
        run: &mut WorkflowRun,
        index: usize,
        error: String,
    ) -> Result<bool, String> {
        let step = &mut run.steps[index];
        step.status = WorkflowStepStatus::Failed;
        step.started_at.get_or_insert(db::ts(db::now_ms()));
        step.ended_at = Some(db::ts(db::now_ms()));
        step.error = Some(error.clone());
        let step_id = step.step_id.clone();
        if step.continue_on_failure {
            self.save(run).await?;
            return Ok(true);
        }
        self.fail(
            run,
            Some(step_id.clone()),
            format!("Step '{step_id}' failed: {error}"),
        )
        .await?;
        Ok(false)
    }

    /// Settles every waiting ask step whose request came back: the answer
    /// is the step's output and the run goes on; an error, a decline or no
    /// answer by its due date fails the step. Safe to call any time; the
    /// state is all in the database, so it also catches answers that came
    /// while the app was closed. Returns how many it settled.
    pub async fn settle_asks(self: &Arc<Self>) -> Result<usize, String> {
        let ids = self.db.call(|conn| waiting_run_ids(conn)).await?;
        let mut settled = 0;
        for id in ids {
            let run_id = id.clone();
            let Some(run) = self.db.call(move |conn| get_run(conn, &run_id)).await? else {
                continue;
            };
            for step in &run.steps {
                let (Some(task_id), WorkflowStepStatus::AwaitingInput) =
                    (step.task_id.as_ref(), step.status)
                else {
                    continue;
                };
                let Some(result) = crate::services::a2a::history::get(&self.db, task_id).await?
                else {
                    continue;
                };
                if result.status == crate::services::a2a::TaskStatus::Running {
                    continue;
                }
                self.settle_ask(&run.id, &step.step_id, &result).await?;
                settled += 1;
            }
        }
        Ok(settled)
    }

    async fn settle_ask(
        self: &Arc<Self>,
        run_id: &str,
        step_id: &str,
        result: &crate::services::a2a::TaskResult,
    ) -> Result<(), String> {
        let id = run_id.to_string();
        let Some(mut run) = self.db.call(move |conn| get_run(conn, &id)).await? else {
            return Ok(());
        };
        if run.status != WorkflowRunStatus::Waiting {
            return Ok(());
        }
        let Some(index) = run.steps.iter().position(|s| {
            s.step_id == step_id
                && s.status == WorkflowStepStatus::AwaitingInput
                && s.task_id.as_deref() == Some(result.task_id.as_str())
        }) else {
            return Ok(());
        };
        let step = &mut run.steps[index];
        step.ended_at = Some(db::ts(db::now_ms()));
        if result.status == crate::services::a2a::TaskStatus::Completed {
            let output = crate::services::a2a::returns::text_of(&result.output);
            step.status = WorkflowStepStatus::Done;
            step.output = Some(output.clone());
            run.status = WorkflowRunStatus::Running;
            self.save(&run).await?;
            log::info!("Workflow run {run_id}: step '{step_id}' got its answer");
            events::emit(
                &self.bus,
                &WorkflowStepDone {
                    workflow_id: run.workflow_id.clone(),
                    run_id: run.id.clone(),
                    step_id: step_id.to_string(),
                    output,
                },
            );
            self.spawn_driver(run.id.clone());
            return Ok(());
        }
        let who = step.asked.clone().unwrap_or_else(|| "they".to_string());
        let error = result
            .error
            .clone()
            .unwrap_or_else(|| format!("The request ended as {}", result.status.as_str()));
        step.status = WorkflowStepStatus::Failed;
        step.error = Some(error.clone());
        if step.continue_on_failure {
            run.status = WorkflowRunStatus::Running;
            self.save(&run).await?;
            self.spawn_driver(run.id.clone());
            return Ok(());
        }
        self.fail(
            &mut run,
            Some(step_id.to_string()),
            format!("Step '{step_id}': no answer from {who}: {error}"),
        )
        .await
    }

    /// Keeps settling ask steps every [`ASK_SETTLE_INTERVAL`] (from
    /// startup)
    pub fn watch_asks(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        self.runtime.spawn(async move {
            loop {
                if let Err(e) = engine.settle_asks().await {
                    log::warn!("Settling workflow ask steps failed: {e}");
                }
                tokio::time::sleep(ASK_SETTLE_INTERVAL).await;
            }
        });
    }
}
