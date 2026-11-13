# Groups (protocol 7)

People on a relay who chose to work together, each group with a signed policy that every member's Nandi enforces on work from the group. The wire contract is `nandi_protocol::group` (nandi-spec); the relay keeps groups, members and invites, never parses a policy, and checks only that its author is an admin and its version is the next one.

## Module map

```text
src-tauri/src/services/
  domain/group.rs        pure: AccessLevel, RequestRules (+ Hours), GroupPolicyText,
                         GroupCopy (the copy file), GroupAccess (access.yaml), Decision, verify
  repos/group_repo.rs    the only writer of .nandi/groups/
  groups/mod.rs          Groups: relay calls, signing, keeping the copies, watch()
  groups/enforce.rs      Enforcer / GroupRules: the decision for one incoming request
  relay/client.rs        group_request (request_id correlation), on_groups, public_keys_of,
                         Registered.visibility -> RelayStatus.visibility, no_shared_group
src-tauri/src/rpc/groups.rs
src/hooks/useGroups.ts, src/components/views/{GroupsView,GroupDetail}.tsx,
src/components/network/{groups-model,group-toast}.ts
```

Every command is one declaration in `rpc/groups.rs`, served over IPC and `POST /api/rpc/{name}`: `list_groups` (`GroupsOverview`), `create_group`, `get_group`, `update_group`, `set_group_policy`, `invite_to_group`, `revoke_group_invite`, `join_group`, `leave_group`, `set_group_role`, `remove_group_member`, `get_group_access`, `set_group_access`.

Without a relay every call answers `RelayUnavailable` (the UI shows `RelayConnectNotice`), except `list_groups` (`connected: false`) and the access level, which is a local file.

## The gold copy

`.nandi/groups/<group-id>.md`, one per group this Nandi is in, written only by `GroupRepository`:

```markdown
---
# This Nandi's copy of the group's signed policy, refreshed from the relay.
# Read-only: admins change it in Network > Groups. Editing it here breaks the signature.
group: g_3f2a9c0d11e4b7a8
name: Finance
version: 3
author: alice-laptop-1a2b3c4d
signature: <ed25519, base64>
updated_at: 1762600000000.0
members:
  - instance_id: alice-laptop-1a2b3c4d
    role: admin
  - instance_id: bob-desktop-5e6f7a8b
    role: member
---

<the policy text, byte for byte as signed>
```

`.nandi/groups/access.yaml` holds my level per group (`fully-automated`, `ask-first`, `questions-only`, `send-me-info`, `none`); a group not listed is `ask-first`, and joining or creating writes `ask-first` explicitly.

Copies are refreshed after every registration (`GroupSignal::Registered`: list, then get each of my groups, drop copies of groups I'm no longer in) and on every `GroupChanged` for that group; then `network://group_changed` (`GroupsChanged { groupId }`, `null` after a reconnect) tells the screens, whose queries live under `['network', 'groups']`. Getting a group pins its policy author's key (`peer_keys::learn`) so enforcement can verify offline. The watcher ignores the folder (nothing is projected).

## The policy format

The same Markdown + YAML frontmatter as `.nandi/policies/`, plus a `requests:` block, parsed by `GroupPolicyText::parse` (the other blocks go through `Policy::parse`, so `tools:` etc. still need `applies_to`):

```yaml
requests:
  approval: always # always | first-time | per-member (my own level decides)
  capabilities: { allow: [review-*], deny: [review-secret] } # globs; deny wins
  daily_limit: 20 # per member per day; 0 is refused
  hours: 'Mon-Fri 08:00-18:00' # local time; outside it requests wait
  always_ask: [send, payment] # capability, glob or a word in it: waits
```

No `requests:` block means `approval: always`.

## Signatures

`GroupPolicy::signed_bytes(group_id, version, text)` signed with this device's key (`DeviceKeys::sign`). The creating Nandi picks the group ID (`g_` + 16 lowercase hex, `nandi_protocol::is_group_id`), so version 1 is signed over the real ID like every later one; a taken ID (`group_invalid`) is retried once with a new one. `set_policy` signs `max(policy_version, policy.version) + 1` and, on `group_policy_stale`, re-reads and signs once more.

Verification (`domain::group::verify`) uses my own public key for my policies, else the author's pinned key (`peer_keys::pinned`). No key, a changed key (`PEER_KEY_CHANGED`), or a signature that doesn't match: **fail closed**, every request through that group is refused with "this group's policy couldn't be verified" and the detail shows the warning.

## Enforcement on incoming work

`Enforcer::decide(requester, capability, now)`:

1. Groups whose copy lists both me and the requester. None: `None`, and nothing changes (company relay trust rules, intake, auto-approve rules).
2. Per group: unverified or unreadable policy → `Refuse`; else `RequestRules::decide` (deny/allow → refuse; daily limit → refuse; outside `hours` → ask; `always_ask` → ask; `approval`) **stricter of** `AccessLevel::decide` (fully automated → auto; ask first → ask; questions only → `ask-me` auto, else refuse; send me info → `ask-me` waits, else refuse; none → refuse).
3. Across groups: the strictest (`Decision::strictest`; refuse > ask > auto). Each reason is prefixed with the group's name.

Where it plugs in:

- **Executor intake** (`a2a/executor.rs`, tasks to my agents, `ask-me` included): `Refuse` → `ReasonKind::NotAccepting` ("Not accepted here. Finance: …"); `Ask` → `intake::GroupSay::Ask`, the item waits in the queue even for trusted peers and auto-approve rules, with the reason as a thread note; `Auto` → `GroupSay::Auto`, a clean request is approved on the queue (so the trust check passes) with "your groups' policies and your access level let it run"; suspicious screening still waits.
- **Gateway** (`gateway/router.rs`, tasks to `<me>/gateway`): `Refuse` → rejected at the policy stage; `Ask` → held for approval (`hold_for_approval`) unless it's `assign-me`, which waits for the owner's Done anyway.
- Requests that went ahead are counted per group, member and day in `group_requests` (SQLite, runtime) for `daily_limit` and `first-time`.
- A held or queued request is an Inbox item like any other, so it can be decided in its Ask chat ([interaction-layer](./interaction-layer.md#inbox-ask-chats)): the chat's decision waits for the owner's confirm and then calls the same `decide_gateway_hold` / `intake::approve` as the button, so a group's "Ask first" still holds it and chat never loosens a policy.

A copy that doesn't read at all is skipped and logged (its members are unknown); the next refresh rewrites it.

## Visibility

`Registered { visibility }` is kept in the client and exposed as `RelayStatus.visibility` (`company` | `groups`, `None` before registering). With `groups`, People shows a note. The relay answers a submission to someone outside my groups with an `Error` `no_shared_group` and the task's `refused` event; the client remembers the error's message and turns that refusal into `no_shared_group: You don't share a group with them on this relay` (after `REFUSED_BY_RELAY`), which `friendly-error.ts` maps to `errors.noSharedGroup` (`NO_SHARED_GROUP` is exported to `bindings.ts`).

**Relay limits per person.** A relay limits groups per person and people per group (`group_limit`; the public relay: 10 and 50, counted per account across its Nandis), and a public relay also messages a day (`daily_limit`, 500). `groups::refusal` words `group_limit` as a per-person limit and turns a `daily_limit` refusal into `daily_limit@<reset ms>: …` (`relay::limits::daily_limit`), which `toastGroupError` shows calmly with the reset time. See "The public relay" in `relay-defaults.md`.

## Owner decisions (2026-11-08)

- Outside a group's `hours:`, a request **waits** for approval; it isn't refused.
- Joining a group at **Fully automated** counts as trusting its members: their clean requests skip the per-agent "Ask" trust rule.
- Group copies in `.nandi/groups/` are part of the gold copy: staged and committed with the rest of the repo, and backed up with it.
- Run-level group policy and a real "Send me info" intake come **later** (the two gaps below).

## Gaps

- **Run-level policy.** A group policy's `tools:`, `connectors:`, `network:` blocks are validated but not applied to the run a group request starts: policies narrow per agent (`services::policies`), and runs carry no record of which request or group started them (the same reason `workflow:` targets are refused). Enforcement is at intake.
- **Send me info.** There is no intake for information on its own yet; this level lets only questions (`ask-me`) through, and they always wait in my queue.
- **Ignore** on an invite lasts for the session only.

## Testing

- `domain::group::tests::*` (parsing and errors, hours, each level, stricter / strictest, copy round trip, signatures)
- `repos::group_repo::tests::*`
- `groups::enforce::tests::*` (no shared group, levels vs policy, across groups, fail closed, limits and first time)
- `groups::tests::*` (fake relay: create signs v1 over its own ID and retries a taken one, set policy and stale retry, join/leave, forged policy, refresh drops old copies)
- `intake::tests::a_group_that_*`
- `relay::tests::group_calls_are_answered_by_request_id_and_fail_on_disconnect` (scripted socket)
- `relay::tests::on_the_real_relay_a_member_verifies_and_enforces_the_policy` (nandi-relay: create, invite, join, verify, enforce, a new version)
- `groups-model.test.ts`
