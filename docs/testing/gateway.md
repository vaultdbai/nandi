# The instance gateway agent

Scope: the per-instance gateway agent defined by `~/.nandi/repo/NANDI.md`. This covers the manifest file, the gateway card at the relay, policy (exposure, rate limit, monthly budget, per-caller policies), the busy queue, approval holds and how they are released, plain-language routing (to a local agent, a workflow, or a federated gateway), clarification, the owner's gateway chat, and the gateway commands and HTTP routes.

Notes that apply to the whole section:

- Code: `src-tauri/src/services/gateway/` (`mod.rs`, `manifest.rs`, `registration.rs`, `policy.rs`, `router.rs`, `holds.rs`, `chat.rs`), `src-tauri/src/commands/gateway.rs`, `src-tauri/src/http/gateway.rs`, `src/hooks/useGateway.ts`, `src/components/gateway/`.
- The gateway never runs work itself. A task addressed to `<instance>/gateway` (capability `gateway`, or empty) goes through the executor hook (`a2a/executor.rs` `is_gateway_task` → `router::handle_gateway_task`), which applies policy, may hold or queue the task, routes it, then dispatches a child task.
- SQLite tables: `gateway_callers` (caller policies), `gateway_usage` (monthly counts), `gateway_holds` (approval holds). NANDI.md is the gold copy for everything else.
- HTTP routes are under `/api/gateway/...` and need `Authorization: Bearer $(cat $NANDI_HOME/web_token)`.
- Events: `gateway://config_changed`, `gateway://caller_updated`, `gateway://rate_limited`, `gateway://hold_changed`.
- The routing model is the default model from Settings. With no usable key, LLM routing and chat fail with "Add an API key in Settings so the gateway can route requests".

## Manifest (NANDI.md)

### GW-01 — NANDI.md created on first launch

- **What:** On start, if `~/.nandi/repo/NANDI.md` is missing, the gateway writes it from a default template. The template's frontmatter has an empty `responsibilities: []` (with a comment saying what it is for), relay off and not discoverable, `max_concurrent: 3`, `rate_limit_per_hour: 30`, `require_approval: [assign-me]` (with a comment: work someone assigns you through your gateway waits for your approval; questions, `ask-me`, don't), empty `expose` and `hide`, federation off, and empty routing preferences. The body has a private `<!-- … -->` note, "I'm {owner}.", "What I can help with" (whose placeholder points at `responsibilities:`) and "What I can't help with". The owner is left empty, so the name is "My Nandi" until onboarding sets it. A `NANDI.md` in the repo is not read.
- **Where:** Background, at app start. Code: `services/gateway/manifest.rs` (`DEFAULT_TEMPLATE`, `ensure_default`, `render_default`), `services/gateway/mod.rs` (`init`).
- **Preconditions:** A fresh `NANDI_HOME`.
- **Steps:** 1. Start with a fresh `NANDI_HOME`. 2. Open `~/.nandi/repo/NANDI.md`. 3. Restart.
- **Expected:** The log reads `Created …/NANDI.md`, and the file matches the template, with `name: "My Nandi"` when there is no owner. After the restart the file is unchanged and nothing is logged.
- **Edge / negative cases:**
  - A `NANDI.md` in the repo root is ignored: NANDI.md is still created from the template.
  - An owner name containing `"` or `\` is escaped in the frontmatter.
  - A second start does not overwrite an existing NANDI.md.
  - Onboarding replaces the greeting "I'm ." or "I'm <old>." with the new name. A body the owner rewrote is left unchanged (`with_greeting`).
- **Automation:** Covered by: Rust `#[cfg(test)]` in `services/gateway/manifest.rs` (`parses_the_default_template`, `creates_the_default_once`, `the_template_names_its_owner`, `the_greeting_follows_the_name_until_the_owner_rewrites_it`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### GW-02 — Manifest round-trip, external edits and live reload

- **What:** NANDI.md is parsed as YAML frontmatter plus a Markdown body.
  - Saving keeps the body byte for byte and keeps frontmatter keys this version doesn't know, in place.
  - Writes are atomic (temp file and rename), and an unchanged file is not rewritten.
  - The gateway checks the file's modification time every 5 s. When you edit the file by hand, it emits `gateway://config_changed`, re-registers the card and refreshes federated peers.
  - An empty `owner.github_handle` is dropped from the file.
  - `responsibilities:` (a list of short strings) is read and written normalized: each trimmed, blanks dropped, repeats ignoring case dropped after the first, each cut to 100 characters, at most 50. The cap is silent, so a hand-edited file always loads.
- **Where:** Code: `manifest.rs` (`split`, `parse`, `serialize`, `merge`, `save`), `mod.rs` (`spawn_background`, `changed_on_disk`), `registration.rs` (`POLL_INTERVAL`).
- **Preconditions:** NANDI.md exists.
- **Steps:** 1. Add an unknown key `custom: 1` to the frontmatter by hand. 2. Save any change from the app (GW-03). 3. Re-open the file. 4. Edit `policy.rate_limit_per_hour` in a text editor and save. 5. Watch `/ws` for 10 s.
- **Expected:**
  - After step 2, `custom: 1` and the body are unchanged.
  - Within about 5 s of step 4, `gateway://config_changed` fires, the log reads `NANDI.md changed; re-registering the gateway`, and Sharing → What you offer shows the new limit.
- **Edge / negative cases:**
  - A frontmatter with no closing `---` logs `NANDI.md is invalid: …: the frontmatter has no closing ---`, and the gateway refuses gateway tasks at the admit stage.
  - Frontmatter that is not a mapping is an error.
  - A file with no opening `---` is treated as all body.
- **Automation:** Covered by: `manifest.rs` (`round_trips_and_keeps_the_body_exactly`, `keeps_unknown_keys`, `handles_missing_or_broken_frontmatter`, `saves_atomically_and_keeps_the_body`, `responsibilities_round_trip_normalized`, `responsibilities_are_capped`) | Layer: unit; api for the reload | Gap: no test for the 5 s poll emitting `config_changed`.
- **Needs relay:** no
- **Status:** Shipped

### GW-03 — Gateway config and manifest API

- **What:**
  - `get_gateway_config` and `set_gateway_config` read and replace the structured part of the frontmatter (`relay`, `policy`, `federation`, `routing`). The identity fields and the body are kept.
  - `get_gateway_manifest` and `set_gateway_manifest` read and write the whole file, frontmatter and body.
  - Every save emits `gateway://config_changed` and re-registers the card in the background.
- **Where:** Code: `commands/gateway.rs` (`get_gateway_config`, `set_gateway_config`, `get_gateway_manifest`, `set_gateway_manifest`), `mod.rs` (`save_manifest`, `validate`). HTTP: `GET`/`PUT /api/gateway/config`, `GET`/`PUT /api/gateway/manifest`. UI: `useSetGatewayConfig`, and `useSetGatewayManifest` (used by What you offer, GW-18).
- **Preconditions:** None.
- **Steps:** 1. `GET /api/gateway/config`. 2. `PUT` it back with `policy.maxConcurrent: 5`. 3. `GET /api/gateway/manifest` and check the body. 4. `PUT /api/gateway/config` with `policy.maxConcurrent: 0`.
- **Expected:**
  - The config JSON is camelCase (`relay.enabled`, `policy.rateLimitPerHour`, `federation.trustedPeers`, `routing.preferences`).
  - After step 2, NANDI.md has `max_concurrent: 5`, the body is unchanged, and `gateway://config_changed` fires.
  - Step 4 fails with "The gateway needs to run at least one task at a time", and the file is unchanged.
- **Edge / negative cases:**
  - If the gateway failed to start (no instance ID), every gateway command returns "The gateway is unavailable".
  - A missing NANDI.md reads as the default template.
- **Automation:** Covered by: `mod.rs` (`serves_the_config_from_nandi_md`) | Layer: api | Gap: no test for the `max_concurrent: 0` validation through the API.
- **Needs relay:** no
- **Status:** Shipped

## Card, registration and status

### GW-04 — Gateway card at the relay: registration, heartbeat and withdrawal

- **What:** With `relay.enabled`, the gateway sends a `GatewayCard` to the relay and re-sends it every 30 s, after every save, and on connect. The card carries:
  - The exposed capabilities: the union of what local agents serve, within `expose` when it lists any, minus `hide`.
  - `agentCount`, `currentLoad` (tasks this instance is Working on as provider), `maxConcurrent`, the owner (with the user slug standing in for a missing handle) and `federationEnabled`.
  - `directDelegation`: whether any local agent takes tasks addressed to it directly.
  - The NANDI.md body, with `<!-- … -->` comments removed, only when `relay.discoverable` is on.
  - `responsibilities`: NANDI.md's `responsibilities:` list, whether or not `relay.discoverable` is on, unless `relay.share_responsibilities` is false (the switch "Show my responsibilities on my card" under What you offer; on by default, and on for older files without it). They are short statements the owner wrote to be found by topic ("who is responsible for SOX audit?"); the body stays private. Cards from older Nandis have none (`#[serde(default)]`), so `gatewayVersion` stays 1.

  A route card `<instance>/gateway` serving `gateway` lets the relay route tasks to the gateway, and makes a gateway-only instance count as online.

  The agent cards registered at the relay follow the same switches (`registration::shared_agent_cards`): none while `relay.enabled` is off, and only the capabilities within `expose` / `hide`, so a hidden agent is not listed. Capability presence follows the agent cards. A change to `relay.enabled`, `expose` or `hide` re-registers them (`GatewayService::share_agents`).

  Turning `relay.enabled` off withdraws the card: the gateway sends `registration::withdrawn_card` (the name and owner only, `acceptsDelegation: false`), and the relay removes the member from its directory and people search until they share again. The first sync after start-up withdraws too, so a card left from before (sharing turned off while Nandi was closed) is removed.

- **Where:** Code: `registration.rs` (`build_card`, `exposed_capabilities`, `shared_agent_cards`, `withdrawn_card`, `route_card`, `card_owner`, `current_load`, `HEARTBEAT_INTERVAL`), `mod.rs` (`sync`, `share_agents`, `card`), `relay/mod.rs` (`local_cards_from`); nandi-relay `server.rs` (`RegisterGateway`), `directory.rs` (`withdraw`).
- **Preconditions:** A relay is connected (RL-01). Local agents have `serves:`.
- **Steps:** 1. `PUT /api/gateway/config` with relay enabled and discoverable on. 2. `curl http://<relay>:8787/instances`. 3. Add a capability to `hide` and save. 4. Turn discoverable off. 5. Turn off "Show my responsibilities on my card". 6. Turn relay enabled off. 7. Quit Nandi, set `relay.enabled: true` then back to `false` by hand while another Nandi watches People, and start it again.
- **Expected:**
  - After step 1, the instance shows a gateway card with sorted, distinct capabilities, and from a second instance `POST /api/rpc/get_network_snapshot` shows the member with the body text.
  - After step 3, the hidden capability disappears from the card and from the agent cards within a few seconds; an agent serving only hidden capabilities is no longer listed.
  - After step 4, the card has no body, but still carries `responsibilities`.
  - After step 5, the card has no `responsibilities`, and people search no longer finds the member by them.
  - After step 6, the member disappears from the other instance's People and from `find_people`, its agents are no longer registered, and remote tasks to `<instance>/gateway` are no longer routed.
  - After step 7, the member stays out of the directory after start-up.
- **Edge / negative cases:**
  - With no relay URL, the card's `endpoint` is empty, and the card is sent on connect.
  - A capability matches `expose` and `hide` case-insensitively.
  - When an agent's capability is hidden, that agent is not counted in `agentCount`.
- **Automation:** Covered by: `registration.rs` (`the_card_offers_the_union_of_local_capabilities`, `counts_working_tasks_as_load`, `responsibilities_are_on_the_card_even_when_not_discoverable`, `agents_are_shared_only_while_sharing_is_on_and_as_exposed`, `the_withdrawn_card_says_who_and_nothing_more`, `responsibilities_stay_off_the_card_when_not_shared`), `manifest.rs` (`responsibilities_are_shared_unless_turned_off`); `relay/tests.rs` (`tasks_reach_a_gateway_only_instance`), nandi-relay `tests/relay.rs` (`the_relay_routes_to_a_registered_gateway_without_a_route_card`), `tests/directory.rs` (`turning_sharing_off_leaves_the_directory_and_people_search`) | Layer: unit + api (relay) | Gap: the start-up withdrawal (step 7) and the agent re-registration on a sharing change are checked live only.
- **Needs relay:** yes
- **Status:** Shipped

### GW-05 — Gateway status and Make reachable (API)

- **What:**
  - `get_gateway_status` returns:
    - `enabled` and `discoverable`;
    - `registered` (the relay took the card);
    - the current card;
    - `activeTasks` (gateway slots in use);
    - `queuedTasks` (gateway tasks Submitted, Delivered or Acknowledged);
    - `lastHeartbeatAt`.
  - `make_gateway_reachable` turns on `relay.enabled` and `relay.discoverable` (it saves only if something changed), re-registers right away and returns the status. This extends NW-04, which is the People banner button that calls it.
  - The UI refetches the status every 30 s, and when `gateway://config_changed`, `gateway://rate_limited` or `task://state_changed` arrives (`useGatewayEvents`).
- **Where:** Code: `commands/gateway.rs` (`get_gateway_status`, `make_gateway_reachable`), `mod.rs` (`status`, `queued_tasks`). HTTP: `GET /api/gateway/status`, `POST /api/gateway/reachable`. UI consumers: the People banner (NW-04), the reachability checklist (RL-06), and the Incoming requests empty state (NW-10).
- **Preconditions:** None. For `registered: true`, a connected relay.
- **Steps:** 1. With the relay off in NANDI.md, `GET /api/gateway/status`. 2. `POST /api/gateway/reachable`. 3. `GET` the status again. 4. `POST` it a second time.
- **Expected:**
  - Step 1 shows `enabled:false, registered:false`.
  - After step 2, NANDI.md has `relay.enabled: true` and `discoverable: true`, and the response has `registered: true` when connected.
  - Step 4 doesn't rewrite the file, and no new `config_changed` fires.
- **Edge / negative cases:** When not connected to a relay, the status reads `registered:false` while `enabled:true`, and the card is sent on connect.
- **Automation:** Covered by: `src/components/views/NetworkView.test.tsx` (the banner) | Layer: api | Gap: no Rust test for `make_gateway_reachable`.
- **Needs relay:** yes
- **Status:** Shipped

### GW-06 — Owner from NANDI.md on the local agent cards

- **What:** On every gateway sync (at start, after a save, after a hand edit), the owner from NANDI.md is put on every local agent card: the display name, and the user slug as the handle. When it changed, the agents are re-registered at the relay. This fixed "Nandi user" and instance IDs showing in reassign candidates and delegated prompts.
- **Where:** Code: `mod.rs` (`refresh_owner`), `registration.rs` (`card_owner`), `capability_index.rs` (`set_owner`). Commits `176bd77`, `4ea9b28`.
- **Preconditions:** Onboarding or Settings › Account has set a display name and slug (TM-01). A relay is connected.
- **Steps:** 1. Change the display name in Settings › Account. 2. From a second instance, list the online agents (`GET /api/relay/agents`).
- **Expected:** The other instance sees the new owner name on this instance's agent cards without a restart.
- **Edge / negative cases:**
  - When not connected, the cards go out on the next connect.
  - When the owner didn't change, nothing is re-registered.
- **Automation:** Covered by: `mod.rs` (`onboarding_puts_the_owner_on_agent_cards`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Inbound policy

### GW-07 — Gateway admission policy for inbound requests (order and typed reasons)

- **What:** This extends RL-15. Every task for `<instance>/gateway` is checked in this order, and the first failure refuses it with a typed reason that is sent to a remote requester (`RejectTask` with `reason_kind`):
  1. The caller row is disabled: "Requests from {who} are turned off" (`PausedOrBlocked`).
  2. The caller instance is paused or blocked (TM-20, TM-21): "Requests from {who} are paused" (`PausedOrBlocked`).
  3. The capability is not exposed (`expose`/`hide`): "'{cap}' isn't offered by this gateway" (`NotAccepting`).
  4. The capability is not in the caller's `allowedCapabilities` (when that list isn't empty): "{who} may not ask for '{cap}'" (`NotAccepting`).
  5. The monthly budget is used up (GW-09).
  6. The hourly rate limit is reached (GW-08).
  7. Otherwise the request is allowed. When the capability is in `require_approval`, it is held (GW-13).

  `{who}` is the requester's owner display name, then the agent name, then "another Nandi", and never a device ID. A request with no capability (plain language) skips checks 3 and 4. The agent picked afterwards takes the child task "on the gateway's word", so the agent's own `accepts_delegation` is not needed.

- **Where:** Code: `policy.rs` (`evaluate`, `is_exposed`), `router.rs` (`handle_gateway_task`, `caller_name`, `reject_message`).
- **Preconditions:** Two instances on a relay. B's gateway has relay enabled. A is allowed past trust, or the request is plain language.
- **Steps:** 1. On B, hide `summarize`. 2. From A, `POST /api/delegation/tasks {"capability":"summarize","input":"x","target":"<B>/gateway"}`. 3. On B, set A's caller row `allowedCapabilities: ["translate"]` and send `research`. 4. On B, disable A's row and send anything.
- **Expected:** A's Request page shows a refusal with the matching sentence and reason kind for each step. On B, each task is Rejected and listed under Incoming requests → Refused (NW-10). No usage is counted for a refused request.
- **Edge / negative cases:**
  - A disabled or paused caller is refused without spending a rate-limit token.
  - Same-instance callers skip the paused and blocked check.
  - Capability matching is case-insensitive.
- **Automation:** Covered by: `policy.rs` (`applies_exposure_and_caller_capabilities`, `every_rejection_has_a_kind_and_plain_words`, `rejects_a_paused_caller_without_spending_a_token`, `a_disabled_caller_is_refused_without_spending_a_token`) | Layer: unit (the policy); api (end to end, two instances) | Gap: no end-to-end test that a remote requester receives each typed reason.
- **Needs relay:** yes
- **Status:** Shipped

### GW-08 — Hourly rate limit per caller, with the `gateway://rate_limited` event

- **What:** Each caller instance has a token bucket sized to `policy.rate_limit_per_hour` that refills continuously over an hour. When it is empty, the request is refused with "Rate limit reached (N requests per hour)" (kind `Failed`), and `gateway://rate_limited` fires with `{callerInstance, capability, limit:"rate_limit", reason}`. A limit of `0` means no limit. Buckets are in memory, so a restart refills them.
- **Where:** Code: `policy.rs` (`RateLimiter::try_take`, `RATE_WINDOW_MS`), `router.rs` (emits `GatewayRateLimited`). Edited on Network → Sharing → What you offer → Edit → "Tasks per caller per hour (0: no limit)" (GW-18), or in Studio.
- **Preconditions:** A second instance can send gateway tasks.
- **Steps:** 1. Set `rate_limit_per_hour: 2`. 2. From A, send three gateway requests quickly. 3. Watch B's `/ws`.
- **Expected:** The third request is refused with "Rate limit reached (2 requests per hour)", and `gateway://rate_limited` fires on B. After about 30 min, one more request passes.
- **Edge / negative cases:**
  - Each caller has its own bucket, so another instance is not affected.
  - Set `0` and the limit is off.
- **Automation:** Covered by: `policy.rs` (`rate_limits_per_caller_and_refills_hourly`) | Layer: unit | Gap: no test that the event is emitted.
- **Needs relay:** yes
- **Status:** Shipped

### GW-09 — Monthly task budget and usage per caller

- **What:** A caller row may set `monthlyTaskBudget`. Each request that gets a slot adds one to `gateway_usage(caller_instance, month YYYY-MM)`. Once the count reaches the budget, requests are refused with "{who} used its N requests for this month" (limit `monthly_budget`, which also fires `gateway://rate_limited`). The callers table shows "N tasks" or "N of B" for the month.
- **Where:** Code: `policy.rs` (`usage_count`, `increment_usage`, `usage`, `month_of`), `commands/gateway.rs` (`get_gateway_usage`). HTTP: `GET /api/gateway/usage/{caller_instance}`. UI: the This month column of `GatewayCallersTable` (`UsageCell`).
- **Preconditions:** A caller row exists (GW-10).
- **Steps:** 1. Set the caller's budget to 1. 2. Send two requests from that caller. 3. `GET /api/gateway/usage/<caller>`.
- **Expected:** The first request runs and the second is refused. The usage response is `{"month":"2026-10","taskCount":1,"monthlyTaskBudget":1}`, and the table shows "1 of 1".
- **Edge / negative cases:**
  - The count resets each UTC month.
  - Refused, held-then-declined or expired requests are not counted, because usage is recorded only after a slot is taken.
  - An empty budget means unlimited.
- **Automation:** Covered by: `policy.rs` (`enforces_the_monthly_budget`, `callers_and_usage_round_trip`) | Layer: unit / api | Gap: no UI test for the usage cell.
- **Needs relay:** yes
- **Status:** Shipped

### GW-10 — Caller policies: created on first contact, edited in "Who may call you" → List

- **What:** This extends TM-17. Each instance that sends a gateway request gets a `gateway_callers` row on first contact, with defaults: any exposed capability, 1 at a time, unlimited budget, enabled. `gateway://caller_updated` fires. The List view of Network → Sharing → Who may call you (`GatewayCallersTable`) has one row per caller with:
  - Caller: the name from the directory, else the owner handle, else "Unnamed Nandi". The device ID is only under Details.
  - Allowed capabilities ("Anything exposed" when empty).
  - Limits ("{max} at a time, budget {budget|unlimited}").
  - This month.
  - An Enabled switch.
  - Edit (pencil): an inline row with Allowed capabilities (comma-separated), "Tasks at a time" (≥ 1) and "Monthly budget" (≥ 0, or empty for unlimited), and Save or Cancel.
  - Remove (trash).
- **Where:** Network → Sharing → Who may call you → List. Code: `src/components/gateway/GatewayCallersTable.tsx`, `src/components/views/SharingView.tsx` (`CallersPanel`), `useGatewayCallers`, `useSetCallerPolicy`, `useRemoveCallerPolicy`, `mod.rs` (`caller_policy`, `set_caller_policy`, `remove_caller_policy`). Commands: `list_gateway_callers`, `set_caller_policy`, `remove_caller_policy`. HTTP: `GET /api/gateway/callers`, `PUT`/`DELETE /api/gateway/callers/{caller_instance}` (the path sets `callerInstance`).
- **Preconditions:** At least one remote instance has sent a gateway request, or a row was made with `PUT`.
- **Steps:** 1. Have A send B a request. 2. On B, open Network → Sharing → Who may call you and choose List. 3. Click Edit (pencil), set Allowed capabilities to `summarize, translate`, Tasks at a time to 2 and the budget to 10, then Save. 4. Turn the switch off. 5. Click Remove.
- **Expected:**
  - After step 1, a row for A appears with no refresh needed (`gateway://caller_updated` invalidates the query).
  - After step 3, the row reads `summarize, translate` and "2 at a time, budget 10".
  - After step 4, A's next request is refused (GW-07 rule 1).
  - After step 5, the row disappears. A's next request creates it again with the defaults.
- **Edge / negative cases:**
  - Save is disabled when Tasks at a time is less than 1, or when the budget is negative or not a number.
  - Duplicate or blank entries in Allowed are dropped (`parseList`, and trimming on the server).
  - `PUT` with a blank instance fails with "Name the caller's instance".
  - Removing an unknown caller fails with "No policy for <id>".
  - Errors show as a toast.
  - With no rows, the empty state reads "No callers yet" and "An instance appears here after its first request."
  - On edit, `created_at` is kept.
- **Automation:** Covered by: `mod.rs` (`caller_policies_round_trip_with_events`), `policy.rs` (`callers_and_usage_round_trip`) | Layer: api; ui-e2e for the table | Gap: no component test for `GatewayCallersTable` or the List view.
- **Needs relay:** yes
- **Status:** Shipped

### GW-11 — Busy queue: gateway slots, per-caller concurrency and the wait limit

- **What:** An allowed request needs a slot within both `policy.max_concurrent` (the whole gateway) and the caller's own `maxConcurrent`. When none is free, it is not refused. It joins a priority line (`SlotQueue` line `gateway`, ordered by `task::priority_of(input)` and then creation time). While it waits:
  - the task is noted at stage `slot` with kind `Queued` and "Waiting for a free slot";
  - each time its place changes, the requester is told the new position;
  - it is checked again every second.

  If no slot frees within 10 minutes (or by the task's own deadline, if sooner), it is refused with "This Nandi was still busy when time ran out" (`TimedOut`). If the requester cancels while waiting, it ends quietly ("Cancelled while it waited for a free slot", and nothing is sent back). Slots are released when the routed work finishes.

- **Where:** Code: `router.rs` (`wait_for_slot`, `QUEUE_WAIT_MS`, `WAIT_POLL`), `mod.rs` (`acquire`, `GatewaySlot` drop, `GATEWAY_LINE`), `a2a/queue.rs` (`SlotQueue`). Visible as `queuedTasks`/`activeTasks` in the status (GW-05) and as Waiting in Incoming requests (NW-10).
- **Preconditions:** Two instances. Set B's `max_concurrent: 1` and a slow agent on B.
- **Steps:** 1. From A, send two gateway requests to B back to back. 2. Watch A's Request page for the second one. 3. Let the first finish.
- **Expected:**
  - The second request shows "Waiting for a free slot" with its position, and starts once the first completes.
  - `GET /api/gateway/status` on B shows `activeTasks: 1` while one waits.
- **Edge / negative cases:**
  - A caller whose own `maxConcurrent` is 1 waits even when the gateway has free slots.
  - When the requester's timeout ends the task, it leaves the line.
  - A higher-priority input is served first.
- **Automation:** Covered by: `mod.rs` (`a_slot_is_held_per_task_within_both_limits`), `a2a/tests.rs` (`a_busy_gateway_queues_the_request_until_a_slot_frees`) | Layer: api | Gap: no test for the 10-minute refusal.
- **Needs relay:** yes
- **Status:** Shipped

## Routing plain-language requests

### GW-12 — Routing: direct matches first, then the model

- **What:** This extends NW-08 (the requester's side). After policy and a slot, the gateway picks a target from its catalog:
  - local agents, keeping only their exposed capabilities;
  - local workflows (with a "N steps: a → b" description);
  - with federation on, the trusted peers' gateways (GW-17).

  It tries these deterministic routes in order:
  1. A named capability goes to the local agent serving it with the lowest load ratio (ties broken by ID), with reason "{agent} serves '{cap}'" and confidence 1.0.
  2. A workflow whose ID or name equals the capability, or the request text, case-insensitively: "The '{name}' workflow matches".
  3. A trusted peer gateway serving the capability (least loaded): "{name}'s gateway serves '{cap}'" (0.9).

  Otherwise the default model gets the public manifest body, the catalog and the request, and returns JSON (`kind` agent, workflow, gateway, reject or clarify, plus `id`, `reason`, `confidence`, `question`). The request text comes from a string input, or from the first of the `request`, `message`, `text`, `prompt`, `query` or `task` fields, or else the JSON itself.

  The model's answer is checked against the catalog. An unknown target, unparsable output, or confidence below 0.6 becomes a clarification question (GW-15). A reject becomes a refusal with kind `NotAccepting` ("This request is outside what this gateway handles" when no reason is given). An empty request with an unserved capability is refused with "Nothing here serves '{cap}'".

  The task is acknowledged to the requester with the routing note: "Routed to {agent}: {reason}", "Passed to {peer}'s gateway: {reason}" or "Running the workflow {name}: {reason}". The routed agent then runs a child task (`gateway_child_id`), and its output, status and tokens become the gateway task's result.

- **Where:** Code: `router.rs` (`DefaultRouter::route`, `route_directly`, `fill_prompt`, `parse_decision`, `request_text`, `requested_capability`, `handle_gateway_task`, `adopt`, `MIN_ROUTE_CONFIDENCE` in `mod.rs`), `a2a/executor.rs` (`gateway_dispatched`).
- **Preconditions:** Two instances. B has a model key and at least one agent with `serves:`, which may have `accepts_delegation` off.
- **Steps:** 1. From A, send `{"capability":"gateway","input":{"text":"Find me papers on CRDTs"},"target":"<B>/gateway"}`. 2. Send `{"capability":"summarize",…}` to `<B>/gateway`. 3. Send text that matches a workflow's name exactly. 4. Remove B's API key and send plain text again.
- **Expected:**
  - Step 1: A sees Acknowledged with "Routed to <agent>: <reason>", then Working, then the result. B's log reads `Gateway task …: Routed to …`.
  - Step 2: no model call is made (direct route).
  - Step 3: the workflow runs (GW-16).
  - Step 4: the task fails at stage `route` with "Add an API key in Settings so the gateway can route requests".
- **Edge / negative cases:**
  - Model output in a code fence is still read.
  - The kind spellings `targetAgent`, `localagent` and similar are accepted.
  - A peer ID with a `/gateway` suffix is accepted.
  - Request text is marked as data, not instructions, in the system prompt.
  - A capability with no local agent and no peer goes to the model with the capability named.
- **Automation:** Covered by: `router.rs` (`routes_five_fixture_requests`, `checks_llm_decisions_against_the_catalog`, `reads_the_request_out_of_the_input`); `a2a/tests.rs` (`gateway_tasks_go_to_the_gateway_handler`, `the_gateway_reaches_agents_that_dont_take_direct_delegation`, `a_gateway_task_gets_a_provider_row_with_the_routing_reason`) | Layer: unit (fake model) / api (two instances plus a model key) | Gap: no end-to-end test with a real model.
- **Needs relay:** yes
- **Status:** Shipped

### GW-13 — Approval holds: "Needs my approval" waits for the owner

- **What:** A request for a capability in `policy.require_approval` (the "Needs your approval" column, NW-09) is held, not refused.
  - A `gateway_holds` row is opened (pending). It records who asked (by name), the capability, a summary of up to 280 characters ending in "…", and an expiry 1 hour out (`APPROVAL_TTL_MS`), or the task's deadline if sooner.
  - `gateway://hold_changed` fires.
  - The task stays Delivered at stage `approval` (`NeedsMyApproval`). The requester sees "Waiting for the owner to approve {who}'s request".

  The gateway waits for one of four outcomes:
  - **Approve:** routing goes on, with the note "Approved by the owner".
  - **Decline:** refused with "The owner declined this request" (`Declined`).
  - **Expiry:** refused with "Nobody approved it in time" (`TimedOut`), and the hold is marked expired.
  - **The requester ends the task:** the hold is marked rejected and nothing is sent back.

  Holds survive a restart: a resumed task finds a decision that is already on the row. Approval is checked only when the request names a capability. A plain-language ask is not held.

- **Where:** Code: `services/gateway/holds.rs` (`open`, `decide`, `pending`, `summarize`, `HoldWaiters`, `HOLD_TTL_MS`), `router.rs` (`hold_for_approval`), `mod.rs` (`list_gateway_holds`, `decide_gateway_hold`). Commands: `list_gateway_holds`, `decide_gateway_hold`. HTTP: `GET /api/gateway/holds`, `POST /api/gateway/holds/{task_id}/decide {"approve":true|false}`.
- **Preconditions:** Two instances. B has `require_approval: [summarize]`, and A may call B.
- **Steps:** 1. From A, send `summarize` to `<B>/gateway`. 2. On B, `GET /api/gateway/holds`. 3. `POST …/decide {"approve":true}`. 4. Repeat steps 1–2 and decline. 5. Repeat and wait more than 1 hour (or send with a short task deadline).
- **Expected:**
  - Step 2 returns one pending hold, with `callerName`, `capability: "summarize"`, `summary` and `expiresAt`.
  - Step 3 returns the hold with `status: approved`, and A's task goes on to Working and a result.
  - Step 4 rejects A's task with "The owner declined this request".
  - Step 5 rejects it with "Nobody approved it in time", and the row is `expired`.
- **Edge / negative cases:**
  - Deciding a hold twice, or one that ended, fails with "This request was already decided, or it ended".
  - A `POST` without `approve` returns "Body needs \"approve\": true or false".
  - Restart B while a hold is pending, then decide it: the resumed task follows the decision.
  - Pending holds are listed oldest first.
- **Automation:** Covered by: `holds.rs` (`a_hold_opens_once_and_is_decided_once`, `pending_holds_are_oldest_first`, `every_status_reads_back_and_maps_to_an_outcome`, `a_decision_wakes_its_waiter`, `summaries_are_short`), `a2a/tests.rs` (`a_capability_needing_approval_waits_for_the_owner`) | Layer: api | Gap: no test for expiry or for restart resume with a decided hold.
- **Needs relay:** yes
- **Status:** Shipped

### GW-14 — Releasing a hold from the Inbox or the Request page

- **What:** Pending holds show in the Home Inbox (kind `gatewayHold`), sorted by expiry. A hold reads "{member} asked you to take on some work" for `assign-me`, "{member} asked you a question" for `ask-me`, "{member} asked for {capability}" for other capabilities (or "{member} sent a request that needs your approval"), with the summary, a deadline, **Approve** and **Decline**. Decline asks first: "Decline {member}'s request?" / "{member} is told you said no. They can send it again." The toasts are "{member}'s request goes on" and "Declined {member}'s request". Opening the row goes to the task's Request page. There, the banner offers the same Approve and Reject, and the toasts are "Approved: {member}'s request goes on" and "Declined {member}'s request". The Inbox refreshes on `gateway://hold_changed`, `task://state_changed` and `nandi://resync`.
- **Where:** Home → Inbox (`src/components/inbox/Inbox.tsx` `GatewayHoldRow`, `src/hooks/useInbox.ts` `useGatewayHolds`, `useDecideGatewayHold`, `inbox-model.ts`). Request page banner (`src/components/requests/RequestBanner.tsx`, `request-model.ts` `isGatewayHold`). RL-06 row 6 ("N requests are waiting for your approval" → Open Inbox) links here.
- **Preconditions:** A pending hold (GW-13).
- **Steps:** 1. Open Home → Inbox. 2. Click Approve on the hold. 3. Create another hold, open its Request page and click Reject.
- **Expected:**
  - The row disappears after Approve, with the toast shown.
  - The Request page banner reads `request.blocker.hold.incoming`, with Approve as the main action and Reject second.
  - The requester sees their task move on, or be refused.
- **Edge / negative cases:**
  - Decided and expired holds are not listed.
  - The Inbox badge counts pending holds.
  - Deciding a hold that just expired shows the server error as a toast.
- **Automation:** Covered by: `src/components/inbox/inbox-model.test.ts` (`lists pending holds by their expiry, and counts them`, `leaves out decided and expired holds`), `src/components/requests/request-model.test.ts` (`isGatewayHold`, banner actions) | Layer: unit; ui-e2e | Gap: no component test for `GatewayHoldRow` or the banner's hold path.
- **Needs relay:** yes
- **Status:** Shipped

### GW-20 — Assigned work lasts until the person ends it (`assign-me`)

- **What:** Work assigned to a person (`assign-me`) is an assignment, not a minutes-long request.
  - **Requester side:** the input is `{text, due?, links?}`. The task's deadline is `input.due` (`YYYY-MM-DD` = end of that day, an RFC 3339 time, or `N days`), else 7 days out, never under an hour. `delegate_to_agent` returns `{status: "assigned", provider, task_id, state, due}` once the person's gateway acknowledged it, or after 10 s without a refusal (`ACCEPT_WINDOW`); the chat isn't blocked. The task stays open; its `delegations` row stays `running`.
  - No timeout or abandon sweep touches it, and it never fails over to someone else. A request to someone offline waits in the queue until they're back.
  - **Provider side:** the hold (GW-13) lasts until the due date, not 1 hour. Once approved, the gateway doesn't route to an agent: it opens a `gateway_assignments` row (open), fires `gateway://assignment_changed`, and acknowledges "On {owner}'s list" with the time left as ETA. It takes no gateway slot while it waits.
  - **Links:** `input.links` (each a URL or `{url, name}`) show under the request as chips labelled by name (else the URL), in a list named "Attached"; clicking one opens it in the browser. Only http(s) links are kept (`file:`, `javascript:` and non-URLs are dropped), each once, at most 10. Files can't travel over the relay: share a link to where the file lives.
  - **Assigning yourself:** work assigned to your own gateway behaves the same: the call returns at once (`state` `delivered` while held for your approval, `acknowledged` once on your list), and the card appears in your own Inbox. Your Done/Decline ends the delegation's history row.
  - The Inbox shows "{member} assigned you this", the request, "Due {date}" (or "Due in …" within a day, "Overdue by …" in red after), a note field, **Done** and **Decline** (asks first).
  - **Declined while held:** declining the held request (before approving it) refuses it; on the requester's side the open `delegations` row ends and the Inbox shows "{member} declined it" (`request_returns`, outcome `declined`), as for a decline after approval.
  - **Done:** the result is `{status: "done", by, note}`. **Decline:** the requester's task is refused with "{owner} declined it: {note}". The requester withdrawing it closes the assignment as cancelled.
  - The result reaches the requester however late: pushed when done, or, if it couldn't be delivered, fetched by the requester's status poll (every 60 s and when the provider comes online; `StatusResponse` carries `result`/`error` once the task ended). The requester's `tasks` and `delegations` rows then end, and `delegation://task_completed` fires.
  - Both sides survive restarts: a resumed gateway task finds its assignment (or its outcome) and skips policy and usage counting.
  - `my_requests` shows open assignments as "assigned, on their list" / "on your list" with `due`, and `overdue: true` once past.
- **Where:**
  - Code: `services/a2a/assignment.rs` (`ASSIGN_ME`, `due_at`, `ACCEPT_WINDOW`, `POLL_EVERY_MS`), `delegator.rs` (`Waited::Accepted`, `assign_locally`), `task.rs` (`on_event`, results kept at the relay), `queue.rs` (the outbox), `history.rs` (`close`), `people.rs` (`requests_from`), `gateway/assignments.rs` (`links_of`, `gateway_assignment_links`), `gateway/router.rs` (`assign`), `gateway/mod.rs` (`list_gateway_assignments`, `close_gateway_assignment`).
  - Commands: `list_gateway_assignments`, `close_gateway_assignment`. HTTP: `GET /api/gateway/assignments`, `POST /api/gateway/assignments/{task_id}/close {"done":true|false,"note"?}`.
  - UI: `src/components/inbox/Inbox.tsx` (`AssignmentRow`), `src/hooks/useInbox.ts` (`useGatewayAssignments`, `useCloseGatewayAssignment`), `inbox-model.ts` (kind `assignment`).
- **Preconditions:** Two instances on one relay (the relay on this build: older relays drop the result from status answers). B's NANDI.md has `require_approval: [assign-me]`. A has Chief of Staff.
- **Steps:**
  1. On A, ask Chief of Staff "Assign Lena: send me the list of SOC 1 evidence owners, due 2026-10-30, with this link: https://example.com/soc1" and approve the call.
  2. On B, approve the hold in the Inbox.
  3. Restart B.
  4. On B, type a note and click **Done**.
  5. Assign another task, approve it on B, stop A, click **Done** on B, then start A.
  6. Assign a third and **Decline** it with a note.
  7. On A, ask Chief of Staff to assign yourself "Book the auditor" and approve the call.
- **Expected:**
  - Step 1: A's chat replies that it's assigned (within about 30 s, before B decides). A's task is Delivered, then Working once approved, with the deadline at the end of 2026-10-30.
  - Step 2: the assignment card appears with the due date and the link under "Attached"; clicking it opens the page in the browser.
  - Step 3: the card is still there.
  - Step 4: the card goes; A's task is Completed with `{status: "done", by: "Lena Fischer", note}`, and its history row is completed.
  - Step 5: within a minute of A starting, its task is Completed with the note.
  - Step 6: A's task is Rejected with "Lena Fischer declined it: {note}".
  - Step 7: the chat replies that it's assigned within a few seconds (held for your approval if your NANDI.md requires it); after approving, the card is in A's own Inbox, and **Done** completes the delegation.
- **Edge / negative cases:**
  - Closing an assignment twice, or one that ended, fails with "This assignment was already closed, or it ended". A `POST` without `done` returns "Body needs \"done\": true or false".
  - Cancelling on A closes B's assignment as cancelled.
  - An unparseable `due` falls back to 7 days; a past one to an hour from now.
  - Assigning yourself: `my_requests` doesn't list it (a local path has no `tasks` row), and after a restart the card stays but closing it doesn't end the delegation's history row.
- **Automation:** Covered by: `a2a/assignment.rs` (due parsing), `gateway/assignments.rs` (rows, order, waiters), `a2a/tests.rs` (`an_assignment_stays_open_until_its_person_ends_it`: early return, sweeps, pushed and polled results; `an_assignment_declined_while_held_comes_back_as_declined`; `assigned_work_waits_for_the_owners_done_or_decline`: assigning yourself returns at once, held or listed, with links), `gateway/assignments.rs` (`links_are_kept_in_order_once`, `only_web_links_are_attached`), `people.rs` (`requests_split_by_direction_and_hide_finished_ones`), `inbox-model.test.ts`, `Home.test.tsx` (`shows work assigned to you, overdue, and marks it done with a note`: also opens an attached link), `relay/protocol.rs` (round trip) | Layer: unit + api + ui; live two-instance run 2026-10-25 | Gap: no automated restart-resume test of an open assignment.
- **Needs relay:** yes
- **Status:** Shipped

### GW-15 — Clarifying an unclear request

- **What:** When routing ends in `NeedsClarification` (unknown target, low confidence, or no request text and no capability), the default question is "Which of the capabilities I offer should handle this, and what exactly do you need?".
  - **Remote caller:** the question goes through the clarification protocol (topic `scope`), so the requester's side shows a question card. Each answer is appended to the request as "(Asked: … Answer: …)", the other input fields are kept, and the request is routed again. Rounds are bounded by the clarifier. If asking fails, the task is refused with kind `NeedsTheirAnswer` at stage `route`.
  - **Local caller:** the gateway task completes with output `{"needsClarification":true,"question":…}`.
- **Where:** Code: `router.rs` (`handle_gateway_task` clarification loop, `with_text`, `DEFAULT_QUESTION`), `services/a2a/clarification/`. Commit `d15685c`.
- **Preconditions:** Two instances. B has a model key, and a vague request that the model can't place.
- **Steps:** 1. From A, ask B's gateway "help". 2. Answer the question on A's side (delegation thread or Inbox). 3. Watch B's routing.
- **Expected:** A gets a question card. After the answer, B logs a routing note, and the chosen agent's input `text` includes the "(Asked: … Answer: …)" line.
- **Edge / negative cases:**
  - Asking the same instance's gateway (for example a local `delegate_task` to your own `<instance>/gateway`) returns the question as the result rather than a question card.
  - An empty answer counts as a failed round.
- **Automation:** Covered by: `router.rs` (`clarified_requests_keep_the_rest_of_the_input`) | Layer: api (two instances plus a model key) | Gap: no end-to-end test of a remote clarification round through the gateway.
- **Needs relay:** yes
- **Status:** Shipped

### GW-16 — Routing to a local workflow, with step progress

- **What:** When the route is a workflow, the gateway runs it for the task (`run_for_gateway`). The run ID is recorded on the gateway task (`task_messages` kind `_workflow_run`), so the Request page shows the run's steps live. Each settled step stores a progress message, "Step '{step}' done ({done}/{total})" with its percent, and sends it to a remote requester. The workflow's last output is the result. A failed run completes the task as Failed with the error.
- **Where:** Code: `router.rs` (`RouteDecision::TargetWorkflow`, `WORKFLOW_RUN_KIND`), `services/workflows.rs` (`run_for_gateway`).
- **Preconditions:** B has a workflow (for example "Weekly report") whose steps can run without manual input.
- **Steps:** 1. From A, send the gateway the text "Weekly report", or the capability equal to the workflow ID. 2. Watch A's delegation thread.
- **Expected:** A sees "Running the workflow Weekly report: The 'Weekly report' workflow matches", progress lines with a percent per step, and then the final output. B's Request page links the workflow run.
- **Edge / negative cases:**
  - A workflow with a manual step waits on it. Check that the requester's timeout (RL-14) doesn't end it first.
  - A step failure fails the task with the step's error.
- **Automation:** Covered by: `services/workflows.rs` (`a_gateway_run_reports_progress_and_returns_the_last_output`) | Layer: api | Gap: no two-instance test of workflow routing.
- **Needs relay:** yes
- **Status:** Shipped

### GW-17 — Federation: passing requests to trusted peers' gateways

- **What:** With `federation.enabled` (the Connection → Details "Forward requests to trusted people" switch, RL-05) and at least one `federation.trusted_peers` entry, the gateway fetches the relay's gateway cards every 60 s and keeps those whose owner matches a trusted peer. A peer matches by handle, or by display name when the entry has no handle, case-insensitively, and never itself. These peers join the routing catalog. A request this instance can't serve itself can be passed to a peer's gateway: "Passed to {peer}'s gateway: {reason}". The peer's result becomes ours. A task that another gateway forwarded (requester agent `gateway`) is never forwarded again, so there are no loops.
- **Where:** Code: `mod.rs` (`refresh_peers`, `catalog`), `registration.rs` (`trusted_cards`, `is_trusted_owner`, `FEDERATION_INTERVAL`), `router.rs` (`TargetRemoteGateway`).
- **Preconditions:** Three instances: A (requester), B (federation on, trusts C) and C (serves `translate`). B has no `translate` agent.
- **Steps:** 1. From A, send `translate` to `<B>/gateway`. 2. Watch B and C.
- **Expected:** B acknowledges with "Passed to C's gateway: C's gateway serves 'translate'". C runs it as a gateway task from B, and A receives C's result.
- **Edge / negative cases:**
  - With federation off, or an empty peer list, peers are cleared and nothing is forwarded.
  - A peer whose card doesn't accept delegation is not a direct route.
  - When C refuses, the refusal comes back to A as a failure of B's child task.
- **Automation:** Covered by: `registration.rs` (`follows_only_trusted_peers`) | Layer: api (three instances) | Gap: no end-to-end federation test, and no test of the no-re-forward rule.
- **Needs relay:** yes
- **Status:** Shipped

## Owner-facing gateway UI

### GW-18 — What you offer: the editor, limits and Advanced (Edit as file)

- **What:** This extends NW-09.
  - **Summary:** "About this Nandi" (the description and the body, rendered as markdown), "What you're responsible for" (the `responsibilities:` as tags, or "None listed yet…"), the three columns (Offered, Needs your approval, Hidden), with each capability and the agents that serve it.
    - When `expose` is empty, the note reads "Everything your agents can do is offered, except what you hide."
    - When the approval column has items, it reads "Requests for these wait for you in the Inbox."
    - The limits line reads "Limits: {max} at a time · {rate} per caller per hour".
  - **Edit:** a form with "One-line description", About (the body, with a hint), "What you're responsible for" (one per line, normalized as in GW-02 by `parseResponsibilities`; the hint says it is shown on your card even when you don't share NANDI.md), one select per capability (Offer, Needs approval or Hide), "Tasks at a time" (≥ 1) and "Tasks per caller per hour (0: no limit)" (≥ 0). Saving writes the whole manifest through `set_gateway_manifest` and shows the toast "Saved what you offer" (`network.offer.saved`), listing the changes.
  - **Advanced:** "Edit as file in Ask" opens Studio on the gateway artifact (`PolicyForm gateway`, with display name, max concurrent, rate limit and require approval for, and the commit message "Update NANDI.md"), plus the gateway chat (GW-19).
  - With no capabilities, the "Choose what to offer" button opens Agents.
- **Where:** Network → Sharing → What you offer. Code: `src/components/network/OfferView.tsx` (`OfferSummary`, `OfferEditor`), `offer-model.ts` (`buildOffer`, `applyOffer`, `offerChanges`, `parseResponsibilities`), `src/components/studio/PolicyForm.tsx`, `ArtifactEditor.tsx`, `src/store/studio-store.ts`.
- **Preconditions:** Local agents with `serves:`.
- **Steps:** 1. Open Network → Sharing → What you offer and click **Edit**. 2. Change the About text, set Tasks at a time to 2, and enter `Risk`, `SOX audit` and `risk` on three lines under What you're responsible for. 3. Click **Save**. 4. Open Advanced → **Edit as file in Ask**. 5. In Studio, change the rate limit and save.
- **Expected:**
  - NANDI.md has the new body, `max_concurrent: 2` and `responsibilities: [Risk, SOX audit]`, unknown keys are kept, and the card re-registers (GW-04). The summary shows the two tags.
  - Studio (Ask) opens on the Policy tab with NANDI.md selected. After saving there, What you offer shows the new rate (through `gateway://config_changed`).
- **Edge / negative cases:**
  - Typing a non-number into the limits keeps the previous value.
  - A load error shows the message in red.
  - Save errors show as a toast.
  - Moving a capability back to Offered removes it from `hide` and `require_approval`.
- **Automation:** Covered by: `src/components/network/offer-model.test.ts`, `src/components/studio/change-summary.test.ts` | Layer: unit; ui-e2e | Gap: no component test for `OfferView` or the Edit as file navigation.
- **Needs relay:** no
- **Status:** Shipped

### GW-19 — Ask the gateway (owner's gateway chat)

- **What:** A chat panel titled "Ask the gateway". You talk to your own gateway as a caller would. It answers from:
  - the public NANDI.md body;
  - the exposed catalog;
  - the queue ("N in progress, M waiting; at most K at a time");
  - "Who else to ask": trusted peers plus every online network member whose gateway takes requests.

  It never runs work, and it points you to **Ask** on a member's card, or to Studio, for that. Each message is answered on its own, with no history sent. The conversation lives only in the panel. Enter sends and Shift+Enter adds a new line. While waiting, the panel shows "The gateway is answering…".

- **Where:** Network → Sharing → What you offer → Advanced → "Ask the gateway". Code: `src/components/gateway/GatewayChat.tsx`, `useGatewayChat`, `services/gateway/chat.rs` (`system_prompt`, `add_network_gateways`, `reply`, `MAX_CHAT_CHARS`), `mod.rs` (`chat`). Command: `gateway_chat`. HTTP: `POST /api/gateway/chat {"message":"…"}` (returns a string).
- **Preconditions:** A model key is set.
- **Steps:** 1. Open Network → Sharing → What you offer → Advanced. 2. In "Ask the gateway", type "What can you help with?" and press Enter. 3. Ask "Who else could translate this?" while a member who serves translate is online.
- **Expected:**
  - The answer reflects the "What I can help with" section and the exposed capabilities.
  - Step 3 names the online member.
  - `<!-- … -->` notes are never quoted.
- **Edge / negative cases:**
  - An empty message is blocked (Send disabled; the API returns "Write a message for the gateway").
  - More than 4000 characters fails with "Messages are limited to 4000 characters".
  - With no key, the alert "The gateway couldn't answer" shows the key error.
  - The panel's history clears when you leave the page.
- **Automation:** Covered by: `chat.rs` (`the_owner_hears_about_everyone_on_their_network_who_takes_requests`, `answers_from_the_manifest_body`) | Layer: api (mock LLM) / manual (real model) | Gap: no component test for `GatewayChat`.
- **Needs relay:** no
- **Status:** Shipped

## Gaps and planned (not testable yet)

- `routing.preferences` (per-capability `preferred`, `excluded` and `strategy`: fastest, cheapest, mostTrusted or highestSuccessRate) is parsed from NANDI.md and saved by the config API, but no router code reads it (`services/gateway/router.rs` uses only load ratio and model choice). Neither the UI nor the router uses it.
- The comment on `gateway_chat` in `services/gateway/mod.rs` says "No command calls it yet (see the merge notes in GatewayChat.tsx)". That is stale: `GatewayChat` is mounted under What you offer → Advanced.
- Approval holds apply only when the request names a capability. A plain-language ask routed to an approval-gated capability's agent is not held (`policy::evaluate` takes `needs_approval` from the requested capability only). Check whether this is intended.
- Rate-limit buckets live in memory (`RateLimiter`), so a restart resets every caller's hourly allowance.
- A per-caller policy UI lives only in the List view. There is no way to pre-create a caller from the UI before their first request (the API allows `PUT`).
- Relay `enabled` and `discoverable` are off by default in `DEFAULT_TEMPLATE`. The fix is the guided Make reachable step (GW-05, NW-04, RL-06), not a changed default.
- Planned (plugin packs): pack policies loaded into `GatewayContext`, with a test that a pack policy can't remove an approval the user's policy requires. Not implemented.
