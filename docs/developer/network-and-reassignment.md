# Wave EF: Network, Reassignment, Manual Steps, and Enterprise Config

What wave EF added on top of the network-first wave (`a2a-delegation.md`,
`interaction-layer.md`): trust down to a whole member, supervisor
reassignment of delegated tasks, workflows that wait days for a person,
and a team-shared `.nandi/llm.yaml`.

## Module map

```text
src-tauri/src/services/
  network/mod.rs           NetworkService: directory snapshot, member diff, events
  team/roster.rs           member profiles, by team or by instance
  team/trust.rs            trusted peers, paused delegations, blocked agents / members
  a2a/reassign.rs          reassign (provider), accept (requester), candidates
  workflows/manual_steps.rs  manual_steps table, pending list, complete_manual_step
  config/repo_config.rs    .nandi/llm.yaml: parse, precedence, harness + native defaults
```

| Command                                                | Route                                                                           |
| ------------------------------------------------------ | ------------------------------------------------------------------------------- |
| `get_network_snapshot`, `refresh_network`              | `POST /api/rpc/get_network_snapshot`, `POST /api/rpc/refresh_network`           |
| `get_member_by_instance`                               | `POST /api/rpc/get_member_by_instance`                                          |
| `set_trust_relationship`                               | `POST /api/rpc/set_trust_relationship`                                          |
| `block_agent_from_member`, `unblock_agent_from_member` | `POST /api/rpc/block_agent_from_member`, `unblock_agent_from_member`            |
| `pause_delegations_from_member`                        | `POST /api/rpc/pause_delegations_from_member`                                   |
| `reassign_task`, `list_reassign_candidates`            | `POST /api/rpc/reassign_task`, `POST /api/rpc/list_reassign_candidates`         |
| `complete_manual_step`, `list_pending_manual_steps`    | `POST /api/rpc/complete_manual_step`, `POST /api/rpc/list_pending_manual_steps` |
| `get_repo_llm_config`, `reload_repo_llm_config`        | `POST /api/rpc/get_repo_llm_config`, `POST /api/rpc/reload_repo_llm_config`     |

The wave spec named the member commands `get_member_profile`,
`block_agent`, and `pause_delegations`; those already existed from wave
final under the names above (and `get_member_profile` is the team-scoped
one), so no duplicates were added.

## Network directory and trust

`NetworkService` asks the relay for its member directory (`QueryDirectory`
→ `Directory`: every member's latest gateway card, `online` and
`last_seen_ms`). The relay keeps that in its SQLite store, so members who
left stay listed, offline, across relay restarts, until the operator revokes
them. The answer becomes the known members (`merge_directory`, persisted in
`relay_state`), and each refresh is diffed:

| Event                          | When                                                    |
| ------------------------------ | ------------------------------------------------------- |
| `network://member_joined`      | an instance seen for the first time                     |
| `network://member_online`      | a known member is back                                  |
| `network://member_left`        | a member went offline (also `network://member_offline`) |
| `network://profile_updated`    | a known member's gateway card changed                   |
| `network://membership_changed` | the member count grew                                   |

Every card the relay holds is listed, discoverable or not: `relay.discoverable`
only decides whether the card carries the NANDI.md `body`. A member who turns
`relay.enabled` off sends `registration::withdrawn_card`, and the relay drops
their card, so they leave the directory and people search until they share
again. Their agent cards go too (`registration::shared_agent_cards`).

### Their key changed

Each `MemberRecord` carries the keys the relay pinned for that member
(`publicKeys`). `refresh_with` passes them to `network::peer_keys`, which
keeps this Nandi's own pin in the `peer_keys` table:

| The directory shows   | What happens                                                            |
| --------------------- | ----------------------------------------------------------------------- |
| Keys for someone new  | Pinned                                                                  |
| The pinned keys       | Nothing                                                                 |
| Other keys            | Kept as the change (`changed_*` columns). Every send to them is refused |
| The pinned keys again | The change is dropped                                                   |

- **Keys from the relay.** The first time this Nandi seals for someone, or opens something from them, without a pin, the client asks the relay (`PublicKeysQuery`) and pins the answer the same way (`peer_keys::learn`). The directory isn't needed: members with no gateway card are covered too.
- **The block.** The relay client's key book (`ClientOptions::peer_keys` = `peer_keys::PinnedKeys`, set in `relay::init`) is asked before `delegate_task`, `reassign`, `reseal_as_author`, `send_result`, `send_peer` and `duty_report`, and for every envelope opened. So every path to that member is covered: ask, assign, delegate, results, clarifications, reassignment, handover. The refusal starts with `PEER_KEY_CHANGED` (exported to `bindings.ts`); `friendly-error.ts` maps it to `errors.peerKeyChanged`.
- **Seen live.** When the client learns of a change (a send, or something from them that didn't open with the pinned key), it emits `network://member_key_changed`, and the profile refetches (`agent-sync.ts`).
- **The profile.** `MemberProfile.key` (`PeerKeyStatus`: `fingerprint`, `changedFingerprint`) feeds `MemberKeyNotice`: the fingerprint, or "Their key changed" with both fingerprints and **Accept new key** (`accept_peer_key`, `POST /api/rpc/accept_peer_key`).
- Fingerprints are `nandi_seal::fingerprint`: the key ID's first 16 hex characters in groups of four.

**Relay down:** `get_network_snapshot` serves the last listing with every
member offline (`mark_offline`), and People says "Showing everyone as last
seen". `refresh_network` answers `RelayUnavailable`. Reading the cached
list is fine; searching it is not (people search is strict, see
`a2a-delegation.md`).

### Availability: busy or away, and who covers

NANDI.md's frontmatter (the gold copy) may say whether the owner is around:

```yaml
availability:
  status: away # available (the default), busy or away
  until: 2026-11-10 # optional: a date (start of that day, local) or an RFC 3339 time
  delegate_to: bob-laptop-1a2b3c4d # optional: the instance ID of who covers
```

- `manifest::parse` checks it (`AvailabilityYaml::into_availability`): an
  unknown status or key, an unreadable `until` or a `delegate_to` that isn't
  an instance ID is an error naming what to fix, like any invalid value.
  `available` drops `until` and `delegate_to`. Writes put `until` back as a
  date when it is midnight here.
- `registration::build_card` copies it to `GatewayCard.availability`
  (`nandi_protocol::Availability`, `#[serde(default)]`).
  `Availability::at(now)` is available again once `until` passed; the relay
  applies it to people search, the UI (`availability-model.ts`) to display.
- The People card and the profile header show it with
  `MemberAvailability.tsx`: "Away until Tue, Nov 10 · Bob covers".
- Redirecting work to the delegate is the relay's routing: a question or
  assignment for someone away goes to their own `delegate_to` (and only
  theirs), with "Alice is away; Bob covers" in the thread. See "Who Gets the
  Work" in `a2a-delegation.md`.

### Responsibilities: finding people by topic

Each person states what they own in their NANDI.md frontmatter:

```yaml
responsibilities: [Risk, SOX audit, SOC 1, billing system, support escalations]
```

- `manifest::normalize_responsibilities` trims each item, drops blanks and
  case-insensitive repeats, cuts each to 100 characters and keeps at most 50.
  It runs on parse and on write and is silent (like the rest of the
  frontmatter, a hand-edited file always loads). The UI mirrors it in
  `offer-model.ts` (`parseResponsibilities`).
- `registration::build_card` copies them to `GatewayCard.responsibilities`
  on every card, **independent of `relay.discoverable`**: they are short
  public statements the owner wrote to be found by, unlike the body. Only
  `relay.share_responsibilities: false` keeps them off (the switch under
  What you offer; on by default).
- The field is `#[serde(default)]`. The relay stores the latest card as JSON
  in its `members` table.
- Readers: the relay's people search (`FindPeople`), `network::known_members`
  (each member's card), People search (`searchMembers` in `useNetwork.ts`),
  the member profile's Gateway tab (`MemberGatewayPanel`). Owners edit them
  in Sharing → What you offer.

Trust lives in SQLite (machine-local, never in the gold copy):

- `trust_relationships`: trusted peer (also NANDI.md `federation.trusted_peers`) and paused delegations
- `blocked_members`: `(instance, agent)` pairs; agent `*` (`trust::ALL_AGENTS`) blocks the whole member

Checks: the executor rejects a task from a paused or blocked instance (or
a blocked requester agent) on arrival; the gateway treats a blocked
instance like a paused one; delegation, failover, and the shared grids
never pick or list blocked agents.

## Reassignment (at the relay)

A provider (Alice) hands a task she's serving to an online teammate (Bob)
who serves the same capability. The relay moves it (relay design step
13); the requester never resubmits, and the task keeps its ID.

```text
Alice (provider)             relay                          requester        Bob
reassign_task ── Reassign ──▶ checks the chain
 (request resealed for Bob)   attempt → reassigned           row follows Bob  task (same ID)
 row → Reassigned ◀ ReassignResult   ── reassigned event ──▶ "Reassigned to Bob by Alice"
 run stopped, task://reassigned                               task://reassigned (requester: true)
```

- `TaskState::Reassigned` is terminal and legal from `Delivered`,
  `Acknowledged`, `Working`, and `InputRequired` (Alice's row).
- The requester's row moves to Bob (`task_state::hand_on`): back to
  `Submitted`, the same ID, a `reassigned` thread message and a
  `reassigned_tasks` row. A delegation still waiting keeps waiting on the
  same ID. Bob sees an ordinary task.
- Loop prevention is the relay's: never the requester, never anyone who
  had the task. The app leaves out of the candidates what it knows of.
- Alice without the opened request (a restart): the relay asks the
  requester to reseal it (`reassign::reseal`), and Bob gets it then.
- A manager's move (`POST /team/tasks/{id}/reassign` at the relay) comes
  to Alice's Nandi as a reseal request, which does the same handoff
  (`reassign::reseal`). Nothing moves until a Nandi that can open the
  request reseals it (decision 4: no team key).
- UI: `ReassignMenu` in `ProviderStatePanel` (shown only when
  `list_reassign_candidates` finds someone); the delegation thread renders
  the `reassigned` message as "Reassigned to Bob by Alice"; the requester
  gets a toast.

## Manual steps (long-running workflows)

```yaml
- id: laptop
  step_type: manual
  instructions: 'Tag the laptop for {{accounts.output}} and paste the tag'
  deadline_hours: 48
```

- Domain: `Step.step_type` (`agent` default, `manual`); `agent` is
  optional for manual steps, which need `instructions` (which may use
  `{{step.output}}`). Existing workflows parse unchanged.
- Executor: reaching a manual step renders its instructions, sets the
  step `awaitingInput` and the run `waiting`, records a `manual_steps`
  row, and emits `workflow://manual_step_ready`. No driver task stays
  alive: the state is in `workflow_runs`, so restarts (or days) change
  nothing. `resume_interrupted` only picks up `running` runs.
- `complete_manual_step(run, step, {output, attachmentIds})`: the text
  (plus where attached files were moved, under
  `~/.nandi/attachments/workflow-<run>-<step>/`) becomes the step's output,
  the run goes `running`, `workflow://manual_step_completed` and
  `workflow://step_done` fire, and a driver resumes it. Cancelling a
  waiting run closes its pending step.
- Surfaces: Work's "Waiting on me" shows a `ManualStepCard` per pending
  step (and hides that run's generic item); the dashboard and companion
  list it as a `manualStep` attention item ("Onboarding needs you"); a
  native notification fires; the ask box context lists it under "Waiting
  on the owner".

## Enterprise config (`.nandi/llm.yaml`)

```yaml
providers:
  - id: bedrock-primary
    kind: bedrock # bedrock | azure | litellm | anthropic | openai | ollama
    region: us-east-1
    profile: acme-dev
    model: us.anthropic.claude-sonnet-5
  - id: gateway
    kind: litellm
    endpoint: https://llm.acme.internal
routing:
  default: bedrock-primary
  fallbacks: [gateway]
```

- No secrets in the file: AWS credentials come from the AWS CLI profile,
  keys from the keyring or environment.
- Precedence per value: environment > repo > local. The environment
  variables consulted are `NANDI_LLM_PROVIDER`, `NANDI_LLM_MODEL`,
  `AWS_PROFILE`, `AWS_REGION`/`AWS_DEFAULT_REGION`, and
  `ANTHROPIC_FOUNDRY_BASE_URL` / `ANTHROPIC_BASE_URL` for Azure / LiteLLM
  endpoints. An agent's own `runtime_config` is more specific than all
  three.
- Harness runs (`runtime_router`): `harness_defaults` supplies the
  provider (between the agent's choice and what the CLI detected), the
  Bedrock profile and region, an Azure endpoint, a LiteLLM endpoint as an
  external Anthropic-compatible gateway, and the model.
- Native runs: `orchestrator::default_model` (used by agents, Studio, the
  ask box, run analysis, the gateway router, and compaction) prefers
  `NANDI_LLM_MODEL`, then a native default provider's model
  (`anthropic` → `claude:…`, `openai` → `gpt:…`, `ollama` → `ollama:…`),
  then settings.
- The file is re-read whenever its mtime changes. **Sync** (Settings →
  Advanced → Enterprise config) re-reads it now, records it in
  `repo_config_cache`, and emits `config://repo_llm_changed`.
- `routing.fallbacks` is validated and shown, but not yet used for
  routing.

## Migration v15

`manual_steps`, `reassigned_tasks`, `repo_config_cache`. Trust tables
are from v10 and unchanged.

## Testing

- `a2a::tests::a_provider_reassigns_a_task_and_the_requester_gets_the_result`:
  three instances on the mock relay, end to end
- `relay::sealed_tests::a_reassigned_task_reaches_the_new_provider_signed_by_the_requester`
  (a real relay); the relay's own rules in `nandi-relay/tests/dispatch.rs`
- `a2a::tests::a_blocked_requester_instance_is_rejected_on_arrival`
- `workflows::tests::a_manual_step_waits_for_a_person_and_survives_a_restart`
- `config::repo_config::tests::*`, `runtime_router::tests::*repo*`
- `db::tests::migrates_v14_to_v15_idempotently`
