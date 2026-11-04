# A2A Delegation (Wave 2A, Track C)

One agent asks another agent, on this instance or a teammate's, to do a task for a named capability (`summarize-paper`). Code: `src-tauri/src/services/a2a/`.

## Flow

```
delegate_to_agent tool / "call" button / delegate_task
  → Delegator::run
      providers = local cards (index, else derived from agents) ∪ relay.query_capability
      ranking::rank_providers (see "Who gets the work"), up to MAX_ATTEMPTS (3) tried in order
      local provider  → Executor::execute in-process (no relay needed)
      remote provider → RelayClient::delegate_task, wait live for TaskCompleted/TaskFailed
                        up to the timeout (default 600s, `local_settings.delegation_timeout_secs`)
                        → no answer yet, last provider, not due: TaskStatus::Running
                          {"waiting": true, "due"}, the row stays open until its due date
                        → another provider to try: TaskStatus::TimedOut, next provider
  → delegations row + delegation:// events + ~/.nandi/local/logs/delegations.log
```

### Due dates: requests last until they're due

Every request to another Nandi has a due date (`Task.deadline`, `assignment::request_due_at`): `input.due` when the request names one, else 7 days for `assign-me`, 2 days for a question (`ask-me`) and 1 day for anything else, never less than an hour.

- **The requester waits live** for the timeout, so a quick answer still comes back in the chat. Past that, if there's no one else to try, the tool returns `status: waiting` with the due date and the row stays open. The answer is kept whenever it arrives: pushed (`TaskService::on_result`, which also closes the `delegations` row), or fetched by the status poll that asks every open, non-live request's provider each minute (`poll_assignments`).
- **Someone offline:** the request waits `Submitted` in the offline queue until they're back, however long, until its due date.
- **The sweeps follow the due date:** `sweep_timeouts` ends an acknowledged request past it as `TimedOut`, and `sweep_abandoned` ends one never picked up as `Abandoned`, each with "…by its due date" and the `delegations` row closed. Rows without a deadline keep the old rules (3× the ETA; 300 s / 90 s while the provider is offline).
- **The receiver's side:** a request held for approval, or waiting for a free slot, waits until the task's deadline (`gateway/router.rs` `wait_deadline`), not 1 h or 10 minutes.
- **Failover** gives the next attempt its own due date the same way.

## Who Gets the Work

When several agents serve a capability, `a2a/ranking.rs` decides. Eligible: serves it, `accepts_delegation`, has room (`current_load < max_concurrent`), and isn't the requester. Eligible cards are ordered by:

1. **My pin** for the capability (`settings.yaml` → `delegation.preferred_providers`, personal: it follows me across my devices, never to a team)
2. **Not unreliable**: under 50% success over at least 3 finished tasks goes last
3. **On this instance** (no relay round trip)
4. **Teammate**: the card's owner is a member of one of my teams
5. Lower `current_load`, then faster average duration (when both have history), then newest `issued_at`

Stats (`ProviderStats`) come from my outbound `delegations` rows of the last 30 days. `RankData::gather` reads pins, teammates, and stats once per pick; a source that fails is left empty.

**Fallback**: with no named target, if the pick rejects the task without doing any work (trust, closed, busy, not delivered) or times out, the next ranked provider gets it. The task keeps its ID (the main row shows the final attempt); each passed-on attempt keeps its own row, `<task_id>-attempt<n>`, with "(passed to the next provider)" in its error. On the wire every retry has a fresh ID (`<task_id>-retry<n>`), so a late reply can't answer it. A target named by the caller (a "call" button pick, the tool's `target`) is never swapped.

**Capability-aware pass (wave A)**: when the capability has a definition (`capabilities_projection`) or registered providers (`capability_providers`, plus what the relay reports through `CapabilityPresenceQuery`), `failover::capability_order` reorders the ranked list by `SelectionService::capability_ranking` (`selection/capability.rs`). A provider must have every tool in `required_tools` + `routing.require_tools` (bare names mean `builtin.<name>`), must not be in `routing.excluded_providers`, and must have a closed provider circuit (`selection/circuit.rs`, `provider_circuits`: three failures in a row open it for five minutes, a success closes it; `task://circuit_opened`). Survivors are scored (capability 25%, trust 20%, success 20%, load 15%, ETA 10%, cost 5%, tool richness 5%), the strategy (`routing.strategy`: `least_loaded` default, `round_robin`, `most_trusted`, `fastest`, `highest_success_rate`) makes its part dominant, and `preferred_providers` get +50%. Ranked workflow providers join the queue without a card (`workflow_provider::try_dispatch` runs them). Cards the ranking never heard of (peers from before wave A, without presence) stay after the ranked ones unless the capability requires tools. The delegator records every attempt's outcome on the provider circuit; failover re-ranks the same way.

**Presence (wave A)**: `relay::presence::publish` sets this instance's providers (enabled `capability_providers` rows, with running tasks as load) and enabled tools on the relay client before it registers, so `RegisterInstance` carries them; `spawn_refresh` sends `UpdatePresence` whenever they change (checked every heartbeat interval). The relay stamps each provider with the instance's connection time and last heartbeat. A relay from before wave A answers presence queries with `bad_message`; the client then stops asking until it reconnects.

**Seeing it**: `list_capability_providers(capability?)` (`services/shared.rs`) returns, per capability, every provider (mine, and those shared with me from teams and the relay) with owner, teammate, sources, stats, its rank (1 = Auto's pick), or why it can't take tasks (`Unavailable`: `offline` for team-registry-only cards, `full`, `closed`, …), plus my pin and Auto's choice. "Shared with me → By capability" draws it; `set_provider_pin(capability, cardId | null)` pins and unpins. The call dialog's "Send to" shows "Auto: will use …" from the same data.

The provider side (`executor.rs`) runs on `RelayClient::on_task_assigned`: it admits the task, enqueues it as a turn on the target agent (`SourceType::Agent`, `PRIORITY_AGENT_DELEGATION`, in the agent's conversation), waits for the turn, and sends the final reply back with `send_result`. JSON replies stay structured; anything else is a string.

## Rules

- **Outbound**: an agent is offered `delegate_to_agent` when its skill.md sets `delegation_outbound: true` or lists the tool in `allowed-tools`. Every call goes through the orchestrator's existing approval path (approval row, `agent://approval_request`, resume on approve) unless skill.md declares `requires_approval` without the tool. The provider is only picked after approval, so local and cross-user calls are treated the same.
- **Inbound** is rejected (the requester sees why) when the agent doesn't accept delegation, doesn't serve the capability, is at `max_delegation_concurrent`, or, for requesters on another instance, the trust policy is `deny`. Trust policy `ask` also rejects, with an error naming the requester, unless the owner marked the requester's instance a trusted peer (Network → member → Trust, `team::trust::is_trusted_peer`) or allows it in the trust graph. Same-instance requesters skip the trust policy.
- **Routed by the gateway**: a task the local gateway routes to an agent is sent as `<instance>/gateway`, and `admit` takes it on the gateway's word (NANDI.md `expose`/`hide` and the caller policy already decided), so the agent needn't set `accepts_delegation` or serve the exact capability. An explicit per-capability `deny` override still wins.
- **Per-capability overrides** (`delegation_overrides: {cap: allow|deny}` in skill.md): `allow` accepts that capability even with `accepts_delegation` off and stands in for a missing trust rule; `deny` rejects it; an explicit trust `deny` still wins.
- Settings are written to skill.md through the repos layer (`settings::apply` → `repos().agents.save` → `commit`), never to SQLite.

## Asking Another Person's Nandi

Most people don't publish individual agents; they publish a gateway (NANDI.md `relay.enabled`, turned on by onboarding when a relay is chosen). A request for someone's Nandi targets `<instance>/gateway` with capability `gateway` and input `{"text": "..."}`. Entry points: "Ask {name}" on the member's profile or card (Network), Studio (`delegate_to_agent`, always behind the user's approval; its prompt lists online members and their targets), and `POST /api/delegation/tasks`.

- **Routing at the relay**: relays route `DelegateTask` to agent cards only, so while the gateway accepts delegation the relay client adds a route card (`<instance>/gateway`, serving `gateway`, `gateway::registration::route_card`) to every `RegisterInstance`. It also makes a gateway-only instance count as online (queue flush, `is_online`). Route cards are left out of "agents online" lists. The relay server also routes to a registered accepting gateway card directly.
- **Lifecycle**: an inbound gateway task gets a provider row like an agent task. Once the gateway routes it, `executor::gateway_dispatched` acknowledges it with the routing reason as the note ("Routed to Bob's Helper: …"), then moves it to `Working`. The provider always sends one "started" `ProgressUpdate` when work begins, and the requester treats progress as `Working` (there is no separate wire message). Cancelling stops the turn of the agent it was routed to (`RUNNING_TURNS`, keyed by task ID).
- **Clarification**: when the router can't route confidently, a caller on another instance is asked through the clarification protocol (`ClarificationService::ask`; their task goes to `inputRequired` with a question card), and the answer is added to the request before routing again. Local callers (gateway chat) get the question as text.
- **Offline**: a task for an offline member stays `Submitted` in the offline queue. A relay refusing to route a send ("No online agent …") only answers that send; it is not a task result. Flushes run one at a time, and the provider ignores a second delivery of a task it is already handling.
- **Hold and queue** (task 5): a capability marked "Needs my approval" opens a `gateway_holds` row and waits (`router::hold_for_approval`, `gateway::holds`, 1 h). A full gateway waits in `GatewayService::slots()` for up to 10 min. Both stay Delivered and tell the requester through a `ProgressUpdate` carrying `reason_kind` (and `queue_position`), which the requester records as a note event, not as `Working`.
- **Agent changes** re-register the local cards at the relay (gathered over 500ms), so turning on `accepts_delegation` needs no restart.

## Finding People and Tracking Requests

Two read-only built-ins let an agent work for its owner on a team (`services/a2a/people.rs`). They are offered only to agents whose skill.md lists them in `allowed-tools`, so no agent gets the user's network by default. They run on the host and never need approval.

- **`find_people {topic?, limit?}`**: who on the user's network is responsible for a topic. The relay searches (`RelayClient::find_people` → `FindPeople`/`PeopleMatches`, ranked in the relay's `src/directory.rs`) over its member directory, online or not, and its members' online agent cards. Without a connected relay the tool answers `RelayUnavailable`: there is no matching from a cached copy (design decision 2). `person_json` turns each `PeopleMatch` into the tool's JSON (unchanged for agents; `you` is set here, and someone busy or away adds `availability: {status, until, covered_by}`).
  - The relay scores each person on:
    - their stated `responsibilities` (NANDI.md, sent on the gateway card) ×5
    - capability names ×3
    - description and agent cards ×2
    - the NANDI.md body, when they share it, ×1
  - The whole topic matches with letters and digits only, so "SOC 1", "soc1" and "SOC-1" are equal. Stop words like "who", "responsible" and "the" are dropped.
  - An empty topic lists everyone.
  - Each result says what matched, online or last seen, and `ask: {target: "<instance>/gateway", capability}`. The capability is `ask-me` when the person serves it, otherwise `gateway`.
  - The result also counts people without stated responsibilities, so the agent can say a list may be incomplete.
- **`my_requests {direction?, open_only?, limit?}`**: requests between the user and other people, from the `tasks` table (`requester_instance` or `provider_instance` is this instance), plus pending `gateway_holds` shown as "waiting for your approval". Names come from known members.

The built-in **Chief of Staff** agent (`services/builtin_agents.rs`, files in `src-tauri/resources/builtin-agents/chief-of-staff/`) uses both and lists `delegate_to_agent` with approval. It serves two capabilities:

- **`ask-me`**: questions about its owner. The gateway routes these straight to it.
- **`assign-me`**: work for its owner. The default NANDI.md has `policy.require_approval: [assign-me]`, so the gateway holds these for the owner.

A person's Chief of Staff therefore answers questions for them, and work assigned to them waits for their approval.

### Assignments last (`a2a/assignment.rs`, `gateway/assignments.rs`)

`assign-me` is the one capability whose tasks outlive the call (`assignment::is_lasting`):

- **Deadline** = the due date (`input.due`, else 7 days), not `DEFAULT_TIMEOUT_SECS`.
- **The caller doesn't wait for the work:** `Delegator::wait` returns `Waited::Accepted` once the provider acknowledged it (or after `ACCEPT_WINDOW` with no refusal); `delegate_request` returns `TaskStatus::Running` and the tool answers `status: assigned`. The task is no longer live, so its result goes to `TaskService::on_result`, which also ends the `delegations` row (`history::close`).
- **Nothing gives up on it:** `sweep_timeouts`/`sweep_abandoned` skip it, and `after_failure` never fails it over (it was meant for that person).
- **Assigning yourself (local gateway):** `Delegator::assign_locally` runs the gateway task in the background and returns `Running` once a `gateway://hold_changed` (pending) or `gateway://assignment_changed` event names the task (`state` `delivered` / `acknowledged`), or after `ACCEPT_WINDOW`; a refusal that comes first is returned as usual. The background task ends the `delegations` row when the owner closes it. There's no `tasks` row on a local path, so `my_requests` lists it from the open `gateway_assignments` instead (`people::self_assigned`: "from you", "on your list"), and a restart loses the waiter (the Inbox card stays; closing it then doesn't end the history row).
- **Links:** `input.links` (`[url | {url, name?}]`) become `AssignmentLink`s in `gateway_assignment_links` (`assignments::links_of`: http(s) only, deduped, at most 10), shown on the Inbox card. The relay carries no files, so a file is shared as a link to where it lives.
- **Provider:** after the hold is approved, `router::assign` opens a `gateway_assignments` row instead of routing; the task waits (no slot) for `close_gateway_assignment` (Done → result `{status, by, note}`, Decline → `Declined` refusal) or the requester's cancel.
- **Late results:** `StatusResponse` carries `result`/`error` once the provider's row ended, and the requester polls open assignments (`poll_assignments`, every `POLL_EVERY_MS` in the sweep and on `relay://agent_online`). The relay parses peer bodies, so it must run a build that knows these fields.

```rust
// ✅ GOOD: a capability that lasts gets its deadline and its wait from `assignment`
deadline: Some(if assignment::is_lasting(&capability) {
    assignment::due_at(&request.input, now as i64) as f64
} else {
    now + self.deps.timeout.as_millis() as f64
}),

// ❌ BAD: a long timeout for everything ties up every caller for days
const DEFAULT_TIMEOUT_SECS: u64 = 7 * 24 * 3600;
```

## Typed Reasons

Every refusal and every wait is typed, so the UI picks the copy and the fix, and never parses English:

```rust
// ✅ GOOD: a refusal says what kind it is and where it happened
return Err(Refusal::new(ReasonKind::NotTrusted, TaskStage::Admit,
    format!("The owner of '{name}' needs to trust {who} before it takes their requests.")));
tasks.transition_with(&id, TaskState::Rejected, Change::refused(&refusal)).await?;

// ✅ GOOD: a wait that doesn't change state is a note event
task_state::note(db, bus, &id, Change::default().kind(ReasonKind::Queued, TaskStage::Slot)).await?;

// ❌ BAD: free text the requests projection has to guess at
tasks.transition(&id, TaskState::Rejected, Some("gateway is busy".into())).await?;
```

- `ReasonKind` / `TaskStage` live in `a2a/task.rs` and are stored on `task_events` (v17). Add a kind there, then add its row to `requests/blocker.rs::from_reason_kind` and its copy under `request.blocker.<kind>.<incoming|outgoing>` in every locale.
- Messages use display names (`delegator::member_name`, `executor::requester_name`), never instance ids. The message is shown only under Details.
- Over the relay, `RejectTask.reason_kind` and `ProgressUpdate.reason_kind` are optional. The provider sends its own view, and the requester flips it with `for_requester()`.

## Audit

`delegations` holds one row per task and side (in-process delegations have one outbound row). The JSONL log records who called whom, the capability, status, duration, and error, never the input or output. Tests pass `AuditLog::at(path)`; the default location is never written in tests.

## Testing

`services/a2a/tests.rs` has a mock relay hub that links two in-process `AppState`s, plus tests for approval, timeouts, rejections, overrides, and fallback. Ranking and stats unit tests are in `ranking.rs`; the overview's in `shared.rs`.
