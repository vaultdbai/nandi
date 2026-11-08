# A2A Delegation (Wave 2A, Track C)

One agent asks another agent, on this instance or a teammate's, to do a task for a named capability (`summarize-paper`). Code: `src-tauri/src/services/a2a/`.

## Flow

```
delegate_to_agent tool / "call" button / delegate_task
  → Delegator::run
      named card        → it alone (the relay may hand a person's work to their own away-delegate)
      my own providers  → local cards (index, else derived from agents), ordered locally
      else              → the relay's plan (RelayClient::route: RouteQuery → RoutePlan)
      up to MAX_ATTEMPTS (3) tried in order (see "Who gets the work")
      local provider  → Executor::execute in-process (no relay needed)
      remote          → RelayClient::delegate_task(task, the rest of the plan): SubmitTask,
                        sealed for every candidate; the relay fails it over itself
                        wait live for the result up to the timeout (default 600s,
                        `local_settings.delegation_timeout_secs`)
                        → no answer yet, still due: TaskStatus::Running {"waiting": true,
                          "due"}, the row stays open; the relay ends it at its due date
  → delegations row + delegation:// events + ~/.nandi/local/logs/delegations.log
```

### Due dates: requests last until they're due

Every request to another Nandi has a due date (`Task.deadline`, `assignment::request_due_at`): `input.due` when the request names one, else 7 days for `assign-me`, 2 days for a question (`ask-me`) and 1 day for anything else, never less than an hour.

- **The requester waits live** for the timeout, so a quick answer still comes back in the chat. Past that, the tool returns `status: waiting` with the due date and the row stays open. The answer is kept whenever it arrives: the relay keeps it until this Nandi acks it (`TaskService::on_result`, which also closes the `delegations` row). Nothing is polled.
- **Someone offline:** the relay keeps the request until they're back, however long, until its due date. The row stays `Submitted` with a note that the relay holds it.
- **The relay ends it at its due date** (relay design step 11): one nobody took up as `Abandoned`, one taken up as `TimedOut`, for both sides (`TaskEvent::Ended`); `TaskService::on_event` ends the row and the `delegations` row ("…by its due date"). An assignment never expires.
- **The receiver's side:** a request held for approval, or waiting for a free slot, waits until the task's deadline (`gateway/router.rs` `wait_deadline`), not 1 h or 10 minutes.

## Who Gets the Work

The app decides **local or network**; the relay decides **who on the network**. The app has no scorer for other people's providers: that lives only in the relay (BUSL), from metadata (design decision 1).

1. **A named card** (a "call" button pick, the tool's `target`) is tried alone and never swapped. Remote: the relay is asked with the card as the target, and may hand a question or assignment to that person's own away-delegate (never anyone else's choice, design decision 3). With the relay reconnecting, a question or an assignment waits in the outbox and goes when the relay is back; anything else answers `RelayUnavailable`.
2. **My pin on someone else's agent** (`settings.yaml` → `delegation.preferred_providers`) goes first while the relay's plan has it free (`Pick::is_free`).
3. **My own providers** win next and run here with no relay: `delegator::local_order` (serves it, `accepts_delegation`, has room, not the requester; my pin, then lower load, then the card ID), reordered by `failover::capability_order` (below), which also adds my workflow providers.
4. **Otherwise the relay plans it** (`a2a/route.rs`): `route::query` sends the kind (`delegate`, `ask` for `ask-me`, `assign` for `assign-me`), the capability and hints: my pin as `preferred` (1.5× at the relay), the members I paused or blocked and every instance this request already failed with as `excluded` (never stored at the relay). The plan's candidates (3 by default) come best first, each with its pinned keys and its reasons. No relay: `RelayUnavailable` (a local provider never waits for it). An empty plan says why (`NoRoute`: nobody serves it, nobody takes requests from other Nandis, nobody can take it now).

The relay's ranking (filters, availability, score, busy ordering, fairness, circuits) is documented in the relay's `README.md` and `src/routing/`.

**Fallback**: with no named target, my own providers are tried in turn when one rejects the task without doing any work; after the last, the relay plans. Each passed-on local attempt keeps its own row, `<task_id>-attempt<n>`, with "(passed to the next provider)" in its error, and the next try has a fresh wire ID (`<task_id>-retry<n>`).

**Failover at the relay** (relay design step 11): a request for someone else goes in one `SubmitTask` with the request sealed for every candidate of the plan (`delegate_task(task, alternates)`). The relay gives it to the next one when the first refuses it (not a declined assignment), doesn't pick it up within 90 s, or (work for an agent) takes more than 3× its ETA. The task keeps its ID; the requester row follows the new provider (`task_state::hand_on`, back to `Submitted`) and the thread gets a `failed_over` message ("Passed to Cy", with why). Named targets and assignments go alone. **Retry** on a failed request (`TaskService::retry`) makes a new attempt from a fresh plan without everyone it went to.

**Why that provider**: each remote attempt keeps the relay's reasons as a `route` task message (`{provider, reasons, redirectedFrom}`, `route::ROUTE_KIND`). The delegation thread shows it as "Why Bob" with the reasons in the user's language (`components/network/RouteReasons.tsx`, `lib/route-reasons.ts`). Reasons are typed codes with parameters (`RouteReason`: `named`, `pinned`, `covers`, `free`, `busy`, `offline`, `away`, `usually_quick`, `dependable`, `lighter_day`, `their_turn`, `trying_again`), each with an i18n key under `routing.reason.*` in every locale. They never carry another member's success rate (design decision 9).

**My own providers, capability-aware**: when the capability has a definition (`capabilities_projection`) or this instance registers providers for it (`capability_providers`), `failover::capability_order` reorders my own providers by `SelectionService::capability_ranking` (`selection/capability.rs`, this instance's rows only). A provider must have every tool in `required_tools` + `routing.require_tools` (bare names mean `builtin.<name>`), must not be in `routing.excluded_providers`, and must have a closed provider circuit (`selection/circuit.rs`, `provider_circuits`: three failures in a row open it for five minutes, a success closes it; `task://circuit_opened`). Survivors are scored (capability 25%, trust 20%, success 20%, load 15%, ETA 10%, cost 5%, tool richness 5%), the strategy (`routing.strategy`: `least_loaded` default, `round_robin`, `most_trusted`, `fastest`, `highest_success_rate`) makes its part dominant, and `preferred_providers` get +50%. Ranked workflow providers join the queue without a card. The delegator records each local attempt's outcome on the provider circuit; remote circuits are the relay's (from its ledger).

**Presence (wave A)**: `relay::presence::publish` sets this instance's providers (enabled `capability_providers` rows, with running tasks as load) and enabled tools on the relay client before it registers, so `RegisterInstance` carries them; `spawn_refresh` sends `UpdatePresence` whenever they change (checked every heartbeat interval). The relay stamps each provider with the instance's connection time and last heartbeat. A relay from before wave A answers presence queries with `bad_message`; the client then stops asking until it reconnects.

**Seeing it**: `list_capability_providers(capability?)` (`services/shared.rs`) returns, per capability, every provider (mine, and those shared with me from teams and the relay) with owner, teammate, sources, my own track record with it (`ProviderStats`, from my outbound `delegations` of the last 30 days: never another member's), its rank (1 = Auto's pick, in `shared::auto_order`: a free pinned one, mine, then the relay's plan, asked once per capability with up to `MAX_CANDIDATES`), the relay's reasons, or why it can't take tasks (`Unavailable`: `offline` for team-registry-only cards, `full`, `closed`, `notNow` for an online one the relay didn't pick, …), plus my pin and Auto's choice. Without the relay my own still rank. "Shared with me → By capability" draws it; `set_provider_pin(capability, cardId | null)` pins and unpins. The call dialog's "Send to" shows "Auto: will use …" from the same data.

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
- **Offline**: the relay keeps a task for an offline member until they're back (`TaskEvent::Accepted { held: true }`). A relay refusing a send ("No online agent …") only answers that send; it is not a task result. The provider ignores a second delivery of a task it is already handling.
- **Hold and queue** (task 5): a capability marked "Needs my approval" opens a `gateway_holds` row and waits (`router::hold_for_approval`, `gateway::holds`, 1 h). A full gateway waits in `GatewayService::slots()` for up to 10 min. Both stay Delivered and tell the requester through a `ProgressUpdate` carrying `reason_kind` (and `queue_position`), which the requester records as a note event, not as `Working`.
- **Agent changes** re-register the local cards at the relay (gathered over 500ms), so turning on `accepts_delegation` needs no restart.

## Sealed Content

Everything one person's Nandi sends another is sealed for the recipient (`nandi-seal-v1`, protocol 4): the relay forwards envelopes and reads only metadata. The crypto happens in one place, the relay client: `services/relay/sealing.rs` (`Sealer`), called by `client.rs`. The rest of the app sends and receives the opened forms in `services/relay/opened.rs` (`PeerMessage`, `DutyState`, `TaskAssigned`, `TaskCompleted`, re-exported by `relay::protocol`) and never sees an envelope.

| Content                                                 | Sealed in                                                   | Purpose                    |
| ------------------------------------------------------- | ----------------------------------------------------------- | -------------------------- |
| A task's input (`DelegateTask`, also gateway requests)  | `SealedTask.envelope`                                       | `request`                  |
| A result's output and error                             | `ResultReport.envelope`                                     | `result`                   |
| A late result (`StatusResponse`)                        | `envelope`                                                  | `result`                   |
| Questions / answers                                     | `ClarificationRequest` / `ClarificationResponse` `envelope` | `clarification` / `answer` |
| Notes, reject and cancel reasons, a failed duty's error | `envelope`                                                  | `note`                     |
| A reassigned task's request                             | `ReassignTask.request`                                      | `request` (resealed)       |

- **Keys.** Sealed for the recipient's pinned keys (`network::peer_keys`); unknown ones are asked of the relay and pinned on first sight. A changed key refuses with `PEER_KEY_CHANGED`, a member with no keys with `PEER_KEY_MISSING`.
- **Opening.** Inbound task frames go through one queue, opened in order. An envelope must be for us, for this task, purpose and (for a request or result) capability, from the sender, within `MAX_ENVELOPE_BYTES` (1 MiB), and signed by the author's pinned key. Anything else is refused with `SEAL_REFUSED`: logged, emitted as `relay://seal_refused` (a toast), and, for a task we asked for, its failure. A duplicate `(task, from, seq)` is dropped.
- **Size.** Content over the cap isn't sent (`ENVELOPE_TOO_LARGE`): files go as links.
- **Reassignment.** The provider's client reseals the request it opened for the new provider (`nandi_seal::reseal`, keeping the requester's signature) and sends it in `Reassign`; the relay delivers it under the same task ID. The new provider checks the requester signed it. After a restart the provider no longer holds the opened request, so none rides along and the relay asks the requester (the author) to seal it again (`Delivery::Reseal` → `RelayClient::reseal_as_author`). A manager's reassignment waits for the provider's Nandi to reseal: there's no team key.
- **Waiting at the relay.** What the relay keeps for someone offline stays sealed. The client acks a delivery only once a handler has it (`InboxSync` after each registration and whenever a handler is added), and drops one it handled already on this connection.
- **Local delegation** (our own agents, in process) never seals.

What the relay operator can and can't see is in the relay's README.

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

The Chief of Staff also browses websites itself (no separate Browser agent). Its `skill.md` declares the visible `browser` pilot with `allow: [interact]`: submitting, secrets, uploads and paying wait for approval, and sign-ins, CAPTCHAs and payments go through `browser_hand_over`. It ships with no allowed sites; a new site asks the user. Only its chats and intake threads get the pilot tools: `agent_pilot_tools` is merged on the chat branch only, and `data_run_allows` drops them for the email and calendar checks (test `a_check_started_run_never_gets_pilot_tools`). Its `skill.md` body stays at 900 words or fewer. `BUILTIN_AGENTS` is the table of built-ins and their files: `seed` writes each one whose `skill.md` is missing, `is_builtin` makes it undeletable, and `position` lists the built-ins first, in table order.

### Assignments last (`a2a/assignment.rs`, `gateway/assignments.rs`)

`assign-me` is the one capability whose tasks outlive the call (`assignment::is_lasting`):

- **Deadline** = the due date (`input.due`, else 7 days), not `DEFAULT_TIMEOUT_SECS`.
- **The caller doesn't wait for the work:** `Delegator::wait` returns `Waited::Accepted` once the provider acknowledged it (or after `ACCEPT_WINDOW` with no refusal); `delegate_request` returns `TaskStatus::Running` and the tool answers `status: assigned`. The task is no longer live, so its result goes to `TaskService::on_result`, which also ends the `delegations` row (`history::close`).
- **Nothing gives up on it:** the relay never expires an assignment and never fails it over (it was meant for that person).
- **Assigning yourself (local gateway):** `Delegator::assign_locally` runs the gateway task in the background and returns `Running` once a `gateway://hold_changed` (pending) or `gateway://assignment_changed` event names the task (`state` `delivered` / `acknowledged`), or after `ACCEPT_WINDOW`; a refusal that comes first is returned as usual. The background task ends the `delegations` row when the owner closes it. There's no `tasks` row on a local path, so `my_requests` lists it from the open `gateway_assignments` instead (`people::self_assigned`: "from you", "on your list"), and a restart loses the waiter (the Inbox card stays; closing it then doesn't end the history row).
- **Links:** `input.links` (`[url | {url, name?}]`) become `AssignmentLink`s in `gateway_assignment_links` (`assignments::links_of`: http(s) only, deduped, at most 10), shown on the Inbox card. The relay carries no files, so a file is shared as a link to where it lives.
- **Provider:** after the hold is approved, `router::assign` opens a `gateway_assignments` row instead of routing; the task waits (no slot) for `close_gateway_assignment` (Done → result `{status, by, note}`, Decline → `Declined` refusal) or the requester's cancel.
- **Late results:** the relay keeps the result until the requester's Nandi acks it, so an assignment done while the requester was away arrives when it's back. Nothing is polled. The relay never expires an assignment.

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

`services/a2a/tests.rs` has a mock relay hub that links two in-process `AppState`s, plus tests for approval, timeouts, rejections, overrides, and fallback. The hub seals and opens through each instance's `Sealer`, so every a2a test runs with sealing on. Against a real relay, `relay/sealed_tests.rs` records every frame through a WebSocket tap and checks no content crosses in the clear (nor lands in the relay's database), plus reassignment, missing keys and a replaced device; `relay/sealing_tests.rs` covers the `Sealer` itself. The ranking's tables are the relay's (`nandi-relay/src/routing/tests.rs`); here, `route.rs` (the query and the picks), `delegator.rs` (local order, retries), `shared.rs` (the overview and my stats), and `relay/tests.rs` (`the_relay_plans_who_gets_a_request`, against a real relay).
