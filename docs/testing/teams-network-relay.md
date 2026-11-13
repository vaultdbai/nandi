# Teams, Network, Sharing, Delegation and Relay

Scope: teams (git registries), local identity, sharing agents (team and relay), trust (policies, trusted peers, pause, block, trust requests), the Network page (People, Groups, Sharing, Connection), groups and their signed policies, member profiles, task reassignment, agent-to-agent delegation and clarification, the relay (in-app client, `nandi relay`, `nandi-relay`), the git remote for `~/.nandi/repo`, the team-shared `.nandi/llm.yaml`, and the HTTP/WebSocket server with its web auth.

Notes that apply to the whole section:

- Teams are **any git URL** today. `services/team/mod.rs` says it plainly: "There is no GitHub (or any other) API: a registry is any git URL." Commit `91c8fad` (feat(auth-remote): remove auth, add git remote) removed GitHub sign-in. The GitHub-org and Entra ID teams that userguide section 9 describes are not in the code, so they are listed under Gaps.
- Branches pushed by Nandi use the prefix `nandi/` (`services/remote/mod.rs` `DEFAULT_BRANCH_PREFIX = "nandi"`). The user guide still says `nandi/share-…`.
- Two-instance testing: run each instance with its own `NANDI_HOME` and `NANDI_RELAY_URL`. To drive an instance over HTTP, send `Authorization: Bearer $(cat $NANDI_HOME/web_token)`. Fixed ports come from `--headless --port N` (`docs/relay-testing.md` in nandi-relay).

## Identity

### TM-01 — Local identity: display name and user slug

- **Duplicate:** tested under ST-13. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

## Teams (git registries)

### TM-02 — Create a team from a registry git URL

- **What:** You create a team by entering any git URL for a shared "registry" repo. Nandi clones it to `~/.nandi/repo/teams/<id>/`. If the registry layout is missing, Nandi scaffolds it and pushes the scaffold to `nandi/<my slug>`, never to the default branch. You are added as Owner.
- **Where:** Network → People → team filter → "Manage teams…" → Teams → **New team** (dialog "Registry git URL"). Code: `src/components/views/TeamView.tsx` (`CreateTeamDialog`), `commands/teams.rs` (`create_team`; its argument is still named `github_org` but takes a URL), `services/team/mod.rs` (`create_team`), `services/team/registry.rs`. HTTP: `POST /api/teams`, `GET /api/teams`.
- **Preconditions:** A user slug is set (TM-01). You have a reachable git remote with push rights, or a local bare repo for testing.
- **Steps:** 1. Open Network → People. 2. In the team select, choose "Manage teams…". 3. Click **New team**. 4. Enter `file:///…/acme-registry.git` or an https URL and click **Create team**.
- **Expected:**
  - The team appears in "My teams" with "Members: 1" and you shown as Owner.
  - The team ID is the last URL segment without `.git`, lowercased, with other characters turned into `-` (`…/acme/nandi-registry.git` gives `nandi-registry`). A team's source is always a git URL; there is no `github` source and no `registryUrl` request field.
  - A `teams` row and a `team_members` row are written, and `~/.nandi/repo/teams/<id>/` exists.
  - The remote gets a `nandi/<slug>` branch ("Set up the Nandi registry") and its default branch is untouched.
  - A `team://changed` event fires.
- **Edge / negative cases:**
  - Creating a team whose ID already exists fails with "You're already in a team called <id>".
  - A non-git URL fails validation.
  - A URL that yields no name fails with "Can't name a team after '<url>'".
  - When the push fails (unreachable remote, no rights), the team row and the checkout are rolled back.
  - Creating a team with no slug set fails.
- **Automation:** Covered by: `services/team/mod.rs` tests (`team_ids_come_from_the_url`, `create_team_scaffolds_an_empty_registry_on_a_nandi_branch`, `create_team_leaves_a_scaffolded_registry_alone`, `create_team_rejects_bad_urls_and_rolls_back_failures`, `me_is_the_identity_slug`), `services/team/registry.rs` (`sync_clones_an_empty_remote_then_pulls`, `push_head_never_pushes_the_default_branch`); `src/components/views/TeamView.test.tsx` | Layer: api (use a local bare repo as the remote) | Gap: none notable.
- **Needs relay:** no
- **Status:** Shipped

### TM-03 — Team detail: members, shared agents, packs, last sync

- **What:** Selecting a team shows its members with roles (Owner/Admin/Member), the agents shared in its registry ("Shared by {name}"), the registry's packs, "Last synced {time}" (or "Not synced yet"), a roster section and a team activity feed.
- **Where:** Network → People → Manage teams… → select a team. Code: `TeamView.tsx` (`TeamDetailPane`, `MembersCard`), `src/components/team/TeamRosterSection.tsx`, `MemberCard.tsx`, `TeamActivityFeed.tsx`, `commands/teams.rs` (`get_team`), `rpc/teams_roster.rs` (`get_team_roster`, `get_member_profile`), `services/team/roster.rs`. HTTP: `GET /api/teams/{team_id}`, `POST /api/rpc/get_team_roster`, `POST /api/rpc/get_member_profile`.
- **Preconditions:** You are in a team (TM-02). For roster stats, at least one member is on the relay and has delegation history with you.
- **Steps:** 1. Open the team. 2. Check the Members, Shared agents and Packs cards. 3. Check the roster cards: shared agents, team count, sent and received counts, and Open profile. 4. Check the activity feed.
- **Expected:**
  - Each card renders.
  - A member with no gateway shows "Not connected".
  - Activity lines read inbound or outbound (`members.activity.*`).
  - The roster is cached for about a minute (`team://roster_changed` when it changes).
- **Edge / negative cases:**
  - An empty registry shows `teams.agents.empty` and `teams.packs.empty`.
  - A malformed agent folder in the registry is skipped with a log warning.
  - Roster load errors show `members.roster.unavailable`.
- **Automation:** Covered by: `services/team/roster.rs` tests (`rosters_join_members_publishers_gateways_and_teams`, `delegation_stats_count_both_directions`, `rosters_are_cached_for_a_minute`, `a_member_without_a_gateway_is_not_connected`); `TeamView.test.tsx` | Layer: api | Gap: no UI test for TeamActivityFeed or MemberCard.
- **Needs relay:** no
- **Status:** Shipped

### TM-04 — Sync team registry

- **What:** Fetches and fast-forwards the team's checkout so that newly merged shares appear and pending shares that were merged leave the pending list.
- **Where:** Team detail → **Sync**. Code: `commands/teams.rs` (`sync_team_registry`). HTTP: `POST /api/teams/{team_id}/sync`.
- **Preconditions:** You are in a team, and a share branch was merged on the remote.
- **Steps:** 1. Merge a `nandi/share-…` branch into the registry's default branch on the remote. 2. Click **Sync**.
- **Expected:** "Last synced" updates. The agent appears under Shared agents, and its "Waiting for a team admin to merge" badge disappears. A `team://changed` event fires.
- **Edge / negative cases:**
  - When the remote is unreachable, the toast is `teams.error.sync` with the git error.
  - An unknown team ID returns an error.
- **Automation:** Covered by: `registry.rs` (`sync_clones_an_empty_remote_then_pulls`), `mod.rs` (`merged_shares_stop_being_pending`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TM-05 — Join (re-sync) a known team

- **What:** Re-syncs a team that already has a row on this machine and lists you as a Member. It cannot join a team by ID alone, because there is no directory service.
- **Where:** API and command only: `commands/teams.rs` (`join_team`), `POST /api/teams/{team_id}/join`. The `useTeams` hook supports a `join` action, but no screen calls it.
- **Preconditions:** The team row exists locally.
- **Steps:** 1. `POST /api/teams/<id>/join` for a known team. 2. Repeat with an unknown ID.
- **Expected:** For the known team, the checkout syncs and a member row exists for you. For the unknown ID, a 400 error says "There's no team <id> here. Add it with its registry's git URL instead."
- **Edge / negative cases:** Joining with no slug set fails.
- **Automation:** Covered by: `mod.rs` (`join_team_needs_a_known_team`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Partial (no UI entry point; new teams come in through TM-02)

### TM-06 — Leave team

- **What:** Forgets the team on this machine. It deletes the team and member rows and removes the checkout folder. Nothing on the remote changes.
- **Where:** Team detail → **Leave**. HTTP: `POST /api/teams/{team_id}/leave`.
- **Preconditions:** You are in a team.
- **Steps:** 1. Click **Leave**.
- **Expected:** The team disappears from the list, `~/.nandi/repo/teams/<id>/` is deleted, and a `team://changed` event fires. The remote is unchanged.
- **Edge / negative cases:**
  - There is no confirmation dialog: one click leaves. Check this is acceptable.
  - Leaving an unknown ID returns an error.
- **Automation:** Covered by: none specific | Layer: api | Gap: no test for leave.
- **Needs relay:** no
- **Status:** Shipped

### TM-07 — Share an agent with a team (push a share branch, pending merge)

- **What:** Copies the agent folder into the registry and updates `index.json` and the publisher file. The change is pushed to `nandi/share-<slug>-<agent>`. Nothing is merged: the share stays pending until a team admin merges the branch.
- **Where:**
  - Team strip on People ("Share an agent with this team"), or Team detail → **Share agent** (`ShareAgentDialog`).
  - Agent menu → "Share with team" (TM-10).
  - Agent sharing dialog (TM-11).
  - Code: `services/team/mod.rs` (`share_agent`, `remember_pending_share`, `pending_shares`), `src/components/team/share-toast.ts`.
  - HTTP: `POST /api/teams/{team_id}/agents/{agent_id}`.
- **Preconditions:** You are in a team, you have your own agent (not vendored from the marketplace), and push access.
- **Steps:** 1. Click **Share agent**. 2. Pick an agent and click **Share**. 3. Look at the toast. 4. Look at the team's agent list.
- **Expected:**
  - The toast reads `teams.shareDialog.pushed` with the branch name and a **Copy branch** action that copies it to the clipboard.
  - The remote has `nandi/share-<slug>-<agent>`.
  - Team detail shows the agent with the "Waiting for a team admin to merge" badge and its "Pushed as branch" hint, stored under the `local_settings` pending-shares key.
  - Sharing again with no change returns `branch: null` and the toast `teams.shareDialog.done`.
- **Edge / negative cases:**
  - Sharing a marketplace-installed agent fails with "…installed from the marketplace. Fork it to share your own copy."
  - When another publisher already shared the same agent ID, sharing fails with "<publisher> already shared an agent called '<id>' with this team".
  - An unknown agent fails with "No agent '<id>'".
  - With no agents of your own, the dialog shows `teams.shareDialog.noAgents`.
- **Automation:** Covered by: `mod.rs` (`shares_push_a_share_branch`, `merged_shares_stop_being_pending`), `registry.rs` (`copy_agent_copies_the_folder_without_local_clutter`, `index_round_trips_and_defaults`); `TeamView.test.tsx`, `src/components/sharing/AgentMenu.test.tsx` | Layer: api | Gap: no UI test checks the Copy branch toast.
- **Needs relay:** no
- **Status:** Shipped

### TM-08 — Stop sharing an agent with a team

- **What:** Removes the agent folder and its index entry, and pushes the change to `nandi/unshare-<slug>-<agent>`. Only the agent's publisher can do this.
- **Where:** Team detail → **Stop sharing**, Shared by me → sharing toggle (with confirmation), or the Agent menu team checkbox. HTTP: `DELETE /api/teams/{team_id}/agents/{agent_id}`.
- **Preconditions:** You shared the agent earlier.
- **Steps:** 1. Click **Stop sharing**. 2. Check the remote branches.
- **Expected:** An unshare branch is pushed. Shared by me shows the toast "Stopped sharing", and the agent disappears from Shared by me once the change is merged and synced.
- **Edge / negative cases:**
  - Unsharing an agent someone else shared fails with "'<id>' was shared by <x>, not you".
  - Errors show `teams.error.unshare`.
- **Automation:** Covered by: none specific in Rust; `SharedByMe.test.tsx` covers the toggle | Layer: api | Gap: no Rust test for unshare.
- **Needs relay:** no
- **Status:** Shipped

### TM-09 — Teams as a filter on People, with the team strip

- **What:** The People team select narrows the directory to members of that team's roster who have an instance ID. It shows a strip with the team name, "Members: N", "Share an agent with this team" and "Manage team".
- **Where:** Network → People → team select. Code: `src/components/views/NetworkView.tsx` (`TeamMembers`, `TeamStrip`).
- **Preconditions:** You are in a team whose members are on the relay.
- **Steps:** 1. Choose a team in the select. 2. Toggle All, Online and Recently active. 3. Click "Manage team".
- **Expected:** Only roster members are listed. With none online, the text is "No one from this team is on the relay right now." (`network.people.teamNobodyOnline`). "Manage team" opens Teams with a back button labelled "People" (`network.people.back`).
- **Edge / negative cases:** A search with no matches shows `network.noMatches`.
- **Automation:** Covered by: `src/components/views/NetworkView.test.tsx` | Layer: ui-e2e | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

## Sharing agents

### TM-10 — Agent menu: Share with team, Callable over the relay

- **What:** Each agent's menu has a "Share with team" submenu with a checkbox per team, and a "Callable over the relay" toggle. Turning either on asks for confirmation first. Relay sharing turns the agent's `accept_inbound` on or off (skill.md `accepts_delegation`).
- **Where:** Agents list or agent page → "…" menu. Code: `src/components/sharing/AgentMenu.tsx`, `src/hooks/useAgentSharing.ts` (`useToggleTeamShare`, `useSetRelaySharing` → `get_agent_delegation`/`set_agent_delegation`).
- **Preconditions:** You have your own agent. For teams, you are in at least one.
- **Steps:** 1. Open the agent menu. 2. Check a team and confirm "Share {name} with {team}?". 3. Toggle "Callable over the relay" and confirm "Let everyone on your relay use {name}?". 4. Turn it off.
- **Expected:**
  - The team checkbox triggers TM-07.
  - The relay toggle shows the toasts `agentMenu.relayOn` and `agentMenu.relayOff`.
  - skill.md `accepts_delegation` flips, and the cards re-register at the relay within about 500 ms.
- **Edge / negative cases:**
  - A marketplace agent shows "Installed from the marketplace: only its owner can share it".
  - With no teams, the submenu shows `agentMenu.noTeams`.
  - Errors show `agentMenu.relayError`.
- **Automation:** Covered by: `src/components/sharing/AgentMenu.test.tsx` | Layer: ui-e2e | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### TM-11 — Agent sharing dialog ("Share {name}")

- **What:** Shows what others see (owner, capabilities from `serves:`, call count and last called), a switch per team, and the relay switch.
- **Where:** Agents list or agent page → "…" menu → "Sharing settings…". Code: `src/components/sharing/AgentSharingDialog.tsx`.
- **Preconditions:** You have an agent.
- **Steps:** 1. Open the dialog. 2. Toggle a team. 3. Toggle the relay switch.
- **Expected:** The same effects as TM-10. An agent with no `serves:` shows `agentSharing.noCapabilities`. With no teams, the dialog shows "You aren't in any team yet." and a Go to Teams link.
- **Edge / negative cases:** A vendored agent shows `agentSharing.notYours` and the team switches are not offered.
- **Automation:** Covered by: none dedicated | Layer: ui-e2e | Gap: no test for AgentSharingDialog.
- **Needs relay:** yes
- **Status:** Shipped

### TM-12 — Shared with me (agents from teams and the relay)

- **What:** Lists other people's agents from your team registries and the relay, merged by card ID, with your own left out and blocked agents filtered.
  - Agents are grouped by owner ("Alice · Agents: 3"). Each group can be folded and has **View profile** and **Trust** (the TrustControls popover).
  - The "From" filter offers Everyone, My teams, Relay and "Used in the last 7 days".
  - Each card has **Call**. "Install locally" is disabled.
- **Where:** Library → Shared tab (`{kind:'library', tab:'shared'}`). Code: `src/components/views/SharedWithMe.tsx`, `src/components/network/OwnerGroupHeader.tsx`, `commands/shared.rs` (`list_agents_shared_with_me`), `services/shared.rs`. HTTP: `POST /api/rpc/list_agents_shared_with_me`.
- **Preconditions:** You are in a team with shared agents, and/or connected to a relay where others accept delegation.
- **Steps:** 1. Open Library → Shared. 2. Switch the From filter. 3. Fold an owner group. 4. Open View profile. 5. Open the Trust popover.
- **Expected:**
  - Cards show owner, capabilities and a Team or Relay source badge.
  - Team-registry-only agents are offline and cannot take tasks.
  - The empty states are `shared.withMe.notConnectedTitle` when there is no network and `shared.withMe.noneTitle` when connected, each with Find people or Go to Network.
- **Edge / negative cases:**
  - Blocked agents and members do not appear.
  - Load errors show `shared.loadError`.
  - Hovering "Install locally" shows the tooltip "Installing a team agent locally isn't available yet".
- **Automation:** Covered by: `src/components/views/SharedWithMe.test.tsx`, `src/hooks/useSharedAgents.test.ts`; Rust `services/shared.rs` (`a_team_shared_agent_shows_up_with_its_team`, `team_and_relay_copies_merge_by_card_id_and_mine_are_left_out`, `blocked_agents_are_filtered_from_listings`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### TM-13 — By capability: provider ranking, Auto and pins

- **What:** For each capability, lists every provider (yours, teammates' and relay agents) in the order delegation would try them: a pinned one the relay has free, then yours (they run here), then the relay's plan. "Auto sends tasks to {name}" names the pick. Other members' providers show the relay's reasons in your language ("Bob is free · Bob is usually quick"), never their success rate. Unavailable providers show the reason: offline, full, closed, not serving, "That's you", or "Online, but not picked right now". **Pin** a provider so Auto uses it first. Pins are personal (`settings.yaml` `delegation.preferred_providers`).
- **Where:** Library → Shared tab → pick a capability in the "Can do" filter; the provider list shows above the cards (`src/components/sharing/CapabilityProvidersList.tsx`, mounted by `SharedWithMe.tsx`; reasons by `components/network/RouteReasons.tsx`). Code: `commands/shared.rs` (`list_capability_providers`, `set_provider_pin`), `services/shared.rs` (`auto_order`), the relay's plan (`a2a/route.rs`). HTTP: `POST /api/rpc/list_capability_providers`, `POST /api/rpc/set_provider_pin {"capability","cardId"|null}`.
- **Preconditions:** At least two providers serve the same capability.
- **Steps:** 1. Open Library → Shared and choose the capability under "Can do". 2. Note the Auto pick. 3. Pin another provider. 4. Unpin it.
- **Expected:**
  - The pinned provider becomes the Auto pick while it is eligible.
  - When the pinned provider is unavailable, the card shows `shared.capabilities.pinUnavailable`.
  - Stats read "Your calls: N · success R" with the average duration: your own calls only.
  - Other members' rows show the relay's reasons; yours show none.
  - **Send task** opens the Call dialog.
- **Edge / negative cases:**
  - Without the relay, your own providers still rank; others show no rank.
  - Errors show `shared.capabilities.pinFailed`.
  - A paused member is never offered.
- **Automation:** Covered by: `services/shared.rs` (`the_overview_ranks_callable_providers_and_marks_the_rest`, `a_paused_member_is_not_offered_as_a_provider`, `availability_names_the_reason`, `my_stats_count_outcomes_and_average_successes`), `a2a/tests.rs` (`my_own_agent_wins_unless_i_pinned_someone_elses`), `SharedWithMe.test.tsx` ("shows who gets the work …", with the reasons), `lib/route-reasons.test.ts`; the ranking itself: relay repo `src/routing/tests.rs` | Layer: api + unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TM-14 — Call dialog (send one task to a shared agent)

- **What:** Sends one delegated task. You pick the capability, "Send to" (Auto, or a named provider that is never swapped) and the input (JSON or plain text). The owner's trust policy decides whether it runs.
- **Where:** Shared with me card → **Call**, or By capability → **Send task**. Code: `src/components/sharing/CallDialog.tsx`, `useCallSharedAgent` → `delegate_task`. HTTP: `POST /api/delegation/tasks`.
- **Preconditions:** A provider is reachable: local, or on the relay and trusting you.
- **Steps:** 1. Click **Call**. 2. Keep Auto and enter `{"text":"hi"}`. 3. Click **Send**. 4. Repeat with a named provider.
- **Expected:**
  - The toast reads "Task {id} sent", and the task shows in the delegation thread and in Home activity.
  - With Auto, the hint reads `shared.autoHint`; with a named provider it reads `shared.pickedHint`.
  - When no one is free, the dialog shows "Auto: no one is free right now".
- **Edge / negative cases:**
  - Non-JSON input is sent as a string (`parseCallInput`).
  - Errors show `shared.callFailed`.
  - A named target that rejects the task is not retried elsewhere.
- **Automation:** Covered by: `useSharedAgents.test.ts` (`parseCallInput`); Rust `a2a/tests.rs` (`a_named_provider_is_never_swapped`) | Layer: api | Gap: no UI test for CallDialog.
- **Needs relay:** yes
- **Status:** Shipped

### TM-15 — Shared by me

- **What:** Lists your agents that accept delegation or are shared to a team. Each shows "N calls in 30 days", last called (or "never called"), "Accepts delegation (relay)" and "Shared to {team}" chips, with search and an "All / Recently used" filter. Each sharing chip can be turned off after a confirmation.
- **Where:** Network → Sharing → Shared (`{kind:'network', tab:'sharing', sharing:'shared'}`). Code: `src/components/views/SharedByMe.tsx`, `commands/shared.rs` (`list_agents_shared_by_me`). HTTP: `POST /api/rpc/list_agents_shared_by_me`.
- **Preconditions:** At least one agent is shared or accepts delegation.
- **Steps:** 1. Open the tab. 2. Search. 3. Turn off a chip and confirm.
- **Expected:**
  - The call counts match inbound `delegations` rows.
  - Turning off relay sharing sets `accept_inbound=false`. Turning off a team chip triggers TM-08.
  - The toast reads "Stopped sharing".
- **Edge / negative cases:**
  - The empty state is `shared.byMe.emptyTitle` with a Go to Teams link.
  - Errors show `shared.byMe.stopFailed`.
- **Automation:** Covered by: `src/components/views/SharedByMe.test.tsx`; Rust `services/shared.rs` (`shared_by_me_sums_calls_by_card_id_and_agent_id`, `call_stats_count_the_window_per_direction`, `shared_by_me_lists_delegating_agents_with_inbound_calls`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Trust

### TM-16 — Trust policies per caller and agent (Allow / Ask / Deny)

- **What:** A machine-local rule per edge decides whether a requester (a card ID, or `*`) may run tasks on a local agent. A specific rule beats `*`, and the default is Ask. Ask rejects remote requesters unless they are trusted peers.
- **Where:** Network → Sharing → Who may call you → Graph (TM-17), or via API. Code: `services/trust.rs` (the `trust_policies` table), `commands/shared.rs` (`list_trust_policies`, `set_trust_policy`). HTTP: `POST /api/rpc/list_trust_policies`, `set_trust_policy` (body `requester`, `providerAgent`, `policy`).
- **Preconditions:** Two instances on a relay. B has an agent with `accepts_delegation`.
- **Steps:** 1. On B, set `{requester:"<A>/<agent>", providerAgent:"x", policy:"deny"}`. 2. Have A call B's agent. 3. Change the rule to allow and call again.
- **Expected:** With deny, the task is Rejected and the requester sees the reason. With allow, it runs. The rule is stored in SQLite, not in the gold copy.
- **Edge / negative cases:**
  - An explicit `deny` beats the per-capability `allow` override.
  - Same-instance requesters skip trust.
  - An invalid policy string is a 400.
- **Automation:** Covered by: `services/trust.rs` (`specific_rules_beat_the_wildcard_and_ask_is_the_default`), `a2a/tests.rs` (`remote_requesters_need_an_allowing_trust_policy`, `an_override_can_open_or_close_one_capability`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### TM-17 — Who may call you: list and trust graph

- **What:** A one-sentence summary ("N trusted people can send you work right away. Everyone else asks first."), then a List/Graph toggle showing either a list of callers or a trust graph.
  - The list (`GatewayCallersTable`) edits or removes per-caller gateway policies.
  - The graph has four node kinds: My agents, Agents I can call, Agents that call me, and Teams.
  - Clicking a call line sets Allow, Ask or Deny for that caller.
- **Where:** Network → Sharing → Who may call you, List/Graph toggle (the graph is `{kind:'network', tab:'sharing', sharing:'callers', callers:'graph'}`). Code: `src/components/views/SharingView.tsx` (`CallersPanel`), `src/components/agent-console/TrustGraph.tsx`, `useSetTrustPolicy`.
- **Preconditions:** You have delegation history or team shares, so the graph has edges.
- **Steps:** 1. Switch to Graph. 2. Click an agent to see its connections. 3. Click an inbound call line and choose Deny. 4. Switch to List and remove a caller.
- **Expected:**
  - The edge label reads "{from} → {to} ({n} calls)".
  - The policy is saved; on failure the toast is `shared.graph.policyFailed`.
  - Outbound edges show "You call this agent; its owner sets the policy."
- **Edge / negative cases:** With nothing to show, the graph displays `shared.graph.empty`.
- **Automation:** Covered by: `src/components/agent-console/TrustGraph.test.tsx` | Layer: ui-e2e | Gap: no test for the list view in SharingView.
- **Needs relay:** yes
- **Status:** Shipped

### TM-18 — Trust requests (refused "Ask" attempts), in Sharing and in the Home Inbox

- **What:** When someone you don't trust is refused because the policy is Ask, the refusal shows as one request per person ("Sam wants to send you work · tried N times") with what they asked for.
  - **Trust** makes them a trusted peer.
  - **Deny** dismisses the request until they ask again.
  - **Deny** with "and block" asks for confirmation, then blocks them.
  - There is no "trust once".
- **Where:** The strip at the top of Network → Sharing (`src/components/network/TrustRequestsStrip.tsx`) and the Home Inbox (`src/components/inbox/Inbox.tsx`, kind `trustRequest`). Code: `services/team/trust_requests.rs`, `rpc/teams_roster.rs` (`list_trust_requests`, `respond_trust_request`). HTTP: `POST /api/rpc/list_trust_requests`, `POST /api/rpc/respond_trust_request {"answer":"trust"|"deny","block":bool}`. Event: `trust://request_received`.
- **Preconditions:** Two instances. B has an agent accepting delegation, B's policy for A is Ask, and A is not trusted.
- **Steps:** 1. A calls B's agent twice. 2. On B, open Network → Sharing. 3. Click **Trust**. 4. A calls again.
- **Expected:**
  - B sees one request with attempts=2, the capability and the agent name, in both the strip and the Inbox.
  - After Trust the request disappears, `federation.trusted_peers` in B's NANDI.md lists A, and A's next call runs.
  - Deny hides the request until A is refused again. Deny with block blocks all of A's agents (`*`).
- **Edge / negative cases:**
  - Paused, blocked or explicitly denied people are not listed.
  - Only trust refusals of incoming tasks count.
  - The strip is hidden when there are no requests.
- **Automation:** Covered by: `services/team/trust_requests.rs` tests (`refusals_fold_into_one_request_per_person`, `trusted_people_are_not_waiting`, `paused_or_blocked_people_are_not_waiting`, `an_explicit_never_is_not_a_request`, `not_now_hides_until_they_ask_again`, `only_trust_refusals_of_incoming_tasks_are_read`, `trust_answers_the_request_and_deny_hides_it`) | Layer: api (two instances) | Gap: no UI test for TrustRequestsStrip or the Inbox trust row.
- **Needs relay:** yes
- **Status:** Shipped

### TM-19 — Member trust switches: Trusted peer

- **What:** Marks a member's instance as a trusted peer. This writes `trust_relationships` and adds them to `federation.trusted_peers` in NANDI.md (an atomic write that keeps everything else), so their tasks pass the Ask policy.
- **Where:** Member profile header → **Trust** popover, or the profile's Trust tab (`src/components/network/TrustControls.tsx`, `src/components/team/MemberTrustPanel.tsx`). Code: `services/team/trust.rs` (`set_trust_relationship`). HTTP: `POST /api/rpc/set_trust_relationship` with body `{"instanceId","update":{"addedAsTrustedPeer":bool,"delegationsPaused":null}}`. Event: `trust://relationship_updated`.
- **Preconditions:** A member is known on the directory.
- **Steps:** 1. Toggle Trusted peer on. 2. Check NANDI.md. 3. Have that member call one of your agents with policy Ask.
- **Expected:** The "Trusted" badge shows, NANDI.md lists the peer, and the call runs.
- **Edge / negative cases:**
  - A missing `federation` section is created.
  - An empty instance ID is rejected.
  - Errors show `members.trust.failed`.
- **Automation:** Covered by: `team/trust.rs` (`trusted_peers_edits_keep_everything_else`, `a_missing_federation_section_is_created`, `the_file_is_written_atomically_and_left_alone_when_missing`, `relationships_upsert_and_checks_read_them`, `service_functions_emit_and_validate`), `a2a/tests.rs` (`a_trusted_peer_passes_the_ask_policy`) | Layer: api | Gap: no UI test for TrustControls.
- **Needs relay:** yes
- **Status:** Shipped

### TM-20 — Pause delegations from a member

- **What:** While paused, your gateway turns the member's tasks away, and delegation, failover and the shared grids never pick their agents.
- **Where:** The Trust popover or Trust tab → "Pause delegations". HTTP: `POST /api/rpc/pause_delegations_from_member {"paused":bool}`.
- **Preconditions:** A known member.
- **Steps:** 1. Pause them. 2. Have them call you. 3. Try to call their agent via Auto. 4. Unpause.
- **Expected:**
  - Their task is rejected on arrival, with a typed reason.
  - Their providers are excluded from your ranking, and the profile shows the "Paused" badge.
  - After unpausing, both directions work again.
- **Edge / negative cases:** Pause survives a restart, because it is stored in SQLite.
- **Automation:** Covered by: `a2a/tests.rs` (`a_paused_requester_instance_is_rejected`, `blocked_agents_and_paused_instances_are_never_picked`), `shared.rs` (`a_paused_member_is_not_offered_as_a_provider`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### TM-21 — Block a member, or one of their agents

- **What:** "Block this member" stores `(instance, *)` in `blocked_members`. Their tasks are rejected the moment they arrive, and none of their agents are used. To block a single agent, use **Block** next to that agent on the member profile's Capabilities tab (`MemberAgentsPanel`), or the block API.
- **Where:** The Trust popover or Trust tab → "Block this member"; member profile → Capabilities → **Block** / **Unblock** for one agent. Code: `services/team/trust.rs` (`block_agent_from_member`, `unblock_agent_from_member`, `ALL_AGENTS`). HTTP: `POST`/`POST /api/rpc/unblock_agent_from_member`.
- **Preconditions:** A known member.
- **Steps:** 1. Block the member. 2. Have them call you. 3. Unblock. 4. Block a single agent and check Shared with me.
- **Expected:**
  - Calls are rejected on arrival, and the badges "Blocked" and "Blocked agents: N" show.
  - A blocked agent disappears from Shared with me and from the provider ranking.
  - Unblocking restores everything, and `lastUpdated` does not move backwards.
- **Edge / negative cases:** A blocked requester agent is rejected even when its instance isn't blocked.
- **Automation:** Covered by: `a2a/tests.rs` (`a_blocked_requester_instance_is_rejected_on_arrival`), `shared.rs` (`blocked_agents_are_filtered_from_listings`), `team/roster.rs` (`profiles_group_agents_and_mark_blocked_ones`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

## Network page (People, Sharing, Connection)

### NW-01 — Network page shell: tabs, status pill, "How networking works"

- **What:** The Network page has three tabs: People, Sharing and Connection. Without a saved relay it opens on Connection. Every tab shows a network status pill (which opens Connection) and a "How networking works" popover with a diagram. Dotted terms have one-line tooltips (`Term`).
- **Where:** Sidebar → Network. Code: `src/components/views/NavPages.tsx` (`NetworkPage`), `src/components/network/NetworkHelp.tsx`. Each tab is a `{kind:'network', tab}` view; retired kinds such as `relay`, `team` or `gateway` are rejected by `parseView` (SH-12).
- **Preconditions:** None.
- **Steps:** 1. On a fresh install, open Network. 2. Open "How networking works". 3. Hover a dotted term. 4. Save a relay and reopen Network.
- **Expected:** On a fresh install the Connection tab is active. After a relay is saved, People is the default. The help lists the `network.help.line*` lines.
- **Edge / negative cases:** Opening Connection or Sharing from the palette lands on that tab.
- **Automation:** Covered by: `src/store/navigation-store.test.ts` (view parsing) | Layer: ui-e2e | Gap: no test for NetworkHelp or the pill.
- **Needs relay:** yes
- **Status:** Shipped

### NW-02 — People directory

- **What:** A card per member in the relay's member directory (`QueryDirectory`; this instance is left out), online first and then by name. The relay keeps members who left, offline with when it last saw them, across its restarts, until the operator revokes them. Each card shows the display name, @handle, presence ("Online", "Seen {time}" or "Not seen yet"), availability when they set one (NW-14), "Capabilities: N" and "Agents: N", and a "Gateway only" tag, with Open profile, **Ask** and Copy ID.
  - With the relay down, People shows the last listing, everyone offline, with the note "Showing everyone as last seen. Connect to your company's relay to see who's around now." **Refresh** then answers "Connect to your company's relay" (RL-36).
  - Search filters the cards by name, handle, description, capabilities and responsibilities (the member's NANDI.md `responsibilities:`, GW-04), so "SOX" finds whoever lists "SOX audit".
  - The filter offers All, Online, and Recently active (last 24 h).
  - **Refresh** re-queries the relay.
- **Where:** Network → People. Code: `NetworkView.tsx` (`MemberGrid`), `src/components/network/MemberCardCompact.tsx`, `src/hooks/useNetwork.ts` (`filterMembers`, `searchMembers`, `RECENTLY_ACTIVE_MS`), `services/network/mod.rs`, `rpc/network.rs` (`get_network_snapshot`, `refresh_network`). HTTP: `POST /api/rpc/get_network_snapshot`, `POST /api/rpc/refresh_network`.
- **Preconditions:** Connected to a relay with at least one other instance whose gateway has `relay.enabled` on, with `responsibilities: [SOX audit]` in its NANDI.md.
- **Steps:** 1. Open People. 2. Search for a name, then for `sox`. 3. Toggle the filters. 4. Click Refresh. 5. Click Copy ID.
- **Expected:**
  - Members appear with their counts. With two instances, each side shows 1 member.
  - Copy ID puts the instance ID on the clipboard (toast `network.copied`).
  - A refresh error shows `network.refreshFailed`.
- **Edge / negative cases:**
  - A member whose gateway has `relay.enabled` on but `discoverable` off still appears (the relay lists every member's card); their card just has no `body`. Their responsibilities are still searchable, unless they turned off "Show my responsibilities on my card".
  - A member who turns `relay.enabled` off leaves the directory (GW-04) and comes back when they share again.
  - A member who closed their laptop stays listed, "Seen {time}". A member the operator revoked (RL-30) disappears on the next refresh.
  - A member who never turned `relay.enabled` on has no gateway card and does not appear.
  - With no matches, the grid shows `network.noMatches`.
- **Automation:** Covered by: `NetworkView.test.tsx` (incl. `shows the last listing, marked as last seen, while disconnected`); Rust `services/network/mod.rs` (`an_empty_relay_gives_an_empty_directory`, `one_member_is_built_from_its_card_and_we_are_left_out`, `the_relay_lists_offline_members_and_drops_revoked_ones`, `snapshots_are_cached_until_invalidated`, `the_service_answers_through_app_state`), `relay/tests.rs` (`the_directory_keeps_members_who_left_and_people_are_found_at_the_relay`); relay repo `tests/directory.rs` (`offline_members_stay_listed_after_a_disconnect_and_a_restart`) | Layer: api (two instances) | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-03 — People empty states and invite

- **What:** When not connected and no members are known, People shows "Connect to your company's relay" (`RelayConnectNotice`, RL-36) with **Open connection settings** (opens Connection). When connected but alone, it shows "You're the only Nandi on this relay." and **Copy invite message**, which copies a message with the invite line: the relay URL and, when the user typed a join code for this relay, `#join=<code>` (RL-38). Never the instance token or the build's own code.
- **Where:** Network → People (`MemberGrid`).
- **Preconditions:** Either no relay, or a relay with only this instance.
- **Steps:** 1. With no relay, open People. 2. Connect to an empty relay. 3. Click Copy invite message.
- **Expected:** The notice renders. `network.invite.messageWithCode` (or `network.invite.message` without a code) with the invite line is copied; nothing on screen shows the code.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `NetworkView.test.tsx` | Layer: ui-e2e | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-04 — "Others can't see you" banner with Make reachable

- **What:** When connected but your gateway isn't enabled or discoverable, People shows the alert "Your Nandi is not visible to the network." with a **Make Reachable** button. The button turns on the gateway's relay listing and then refreshes.
- **Where:** Network → People (`data-testid="network-not-reachable"`). Code: `useMakeGatewayReachable` (`src/hooks/useGateway.ts`).
- **Preconditions:** Connected, with gateway `relay.enabled` or `discoverable` off.
- **Steps:** 1. Open People. 2. Click **Make Reachable** in the alert.
- **Expected:** When the relay confirms the registration, the toast is `network.reachable.done`; otherwise it is `network.reachable.pending`. The banner then disappears.
- **Edge / negative cases:** Errors show `network.reachable.failed`.
- **Automation:** Covered by: `NetworkView.test.tsx` | Layer: ui-e2e | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-05 — Network directory events and member diffing

- **What:** Each refresh takes the relay's member directory (online flags and last-seen times come from the relay) as the known members persisted in `relay_state`, diffs it against the previous ones and emits events: `network://member_joined`, `network://member_online`, `network://member_left` (also `network://member_offline`), `network://profile_updated` and `network://membership_changed`. The UI reloads when these arrive.
- **Where:** `services/network/mod.rs`. Events are on the bus, `/ws` and the Tauri webview.
- **Preconditions:** Two or more instances.
- **Steps:** 1. Subscribe to `/ws`. 2. Start instance B. 3. Stop B and wait more than 90 s. 4. Change B's gateway card.
- **Expected:** You see joined, then offline/left, then profile_updated events in that order. The People cards update without a manual refresh.
- **Edge / negative cases:**
  - With no relay configured, init still runs and the directory is empty.
  - When the relay goes down, every known member goes offline (member_left events) and stays listed.
  - Someone who joined and left before you looked joins offline (joined, no member_online).
- **Automation:** Covered by: `network/mod.rs` (`new_members_join_and_changed_cards_update_profiles`, `the_relay_lists_offline_members_and_drops_revoked_ones`, `init_runs_without_a_relay_configured`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-06 — Member profile

- **What:** The profile header shows avatar, name, presence ("Online", "Seen {time}", or "This member hasn't connected their Nandi yet") and the Trusted, Paused, Blocked and "Blocked agents: N" badges, plus a Trust popover. Below it are the diagnosis (NW-07), Ask (NW-08) and four tabs: Gateway (their NANDI.md), Capabilities (with their agents), Recent delegations (both directions) and Trust.
- **Where:** People card → Open profile; Shared groups → View profile; the delegation thread side panel. Code: `src/components/views/MemberProfileView.tsx`, `src/components/team/Member*Panel.tsx`, `rpc/teams_roster.rs` (`get_member_by_instance`). HTTP: `POST /api/rpc/get_member_by_instance`.
- **Preconditions:** A member known on the directory.
- **Steps:** 1. Open a profile. 2. Visit each tab. 3. Click the back button "People".
- **Expected:** Each tab renders. Under the presence line the header shows the member's availability when they set one (NW-14). The Gateway tab lists "Responsible for" with the member's `responsibilities` as tags when the card has any, even when their NANDI.md body isn't shared. A member without a gateway shows the empty state `members.notConnectedDescription` and no diagnosis or Ask.
- **Edge / negative cases:**
  - An unknown instance gets an empty-state profile, not an error.
  - Load errors show `members.loadError`.
- **Automation:** Covered by: `src/components/views/MemberProfileView.test.tsx`; Rust `team/roster.rs` (`an_unknown_instance_gets_the_empty_state`, `profiles_group_agents_and_mark_blocked_ones`) | Layer: ui-e2e | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-07 — "Can {name} receive from me?" diagnosis with a copyable message

- **What:** Two checklists on the profile.
  - **Send:** online, listed, takes general requests, takes requests by name, trusts me, not paused. "Trusts me" is read from how your last request to them ended, and is unknown until you have asked.
  - **Receive:** I'm reachable, I trust them (also passes when **Trust everyone on this relay** is on and the relay needs a token, RL-34), not paused by me.
  - **Copy a message to <name>** builds a note telling the member exactly what to turn on.
- **Where:** Member profile. Code: `src/components/network/MemberDiagnosis.tsx`, `member-diagnosis.ts`.
- **Preconditions:** A connected member.
- **Steps:** 1. Open the profile of a member whose gateway is off for direct requests. 2. Read the checks. 3. Click "Copy a message to <name>".
- **Expected:** Failing rows show "Not yet". The copied text starts with `network.diagnosis.message.hi` and lists one line per failing step.
- **Edge / negative cases:** After a trust refusal from them, "trusts me" shows as failed.
- **Automation:** Covered by: `src/components/network/member-diagnosis.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-08 — Ask {name} (a plain-language request to a member's gateway)

- **What:** A composer sends `{text}` as capability `gateway` to the target `<instance>/gateway`. The member's gateway routes it to one of their agents. Example chips fill in prompts from their capabilities. The card's **Ask** is disabled with a reason when the member is not connected, offline, or not accepting.
- **Where:** Member profile (`src/components/network/AskMemberComposer.tsx`), the People card Ask button. Code: `ask-model.ts` (`gatewayRequest`), `delegate_task`. HTTP: `POST /api/delegation/tasks {"capability":"gateway","input":{"text":"…"},"target":"<instance>/gateway"}`.
- **Preconditions:** The member is online with their gateway accepting delegation, and they trust you, or their policy allows you.
- **Steps:** 1. Type a request and click Send. 2. Follow the opened delegation thread.
- **Expected:**
  - The toast reads "Request sent to {name}" (`members.ask.sent`) and the thread opens.
  - On the provider side, the task is acknowledged with the routing reason ("Routed to Bob's Helper: …"), then moves to Working and returns a result.
- **Edge / negative cases:**
  - When the member is offline, the relay keeps the request and gives it to them when they're back (RL-43).
  - With no relay connected, the composer shows "Connect to your company's relay" (RL-36) instead of Send; a send that answers `RelayUnavailable` shows it too, with no error toast.
  - An untrusted requester is refused with a typed NotTrusted reason, which creates a trust request on their side.
  - A capability marked "Needs my approval" waits on a gateway hold.
- **Automation:** Covered by: `src/components/network/ask-model.test.ts`; Rust `a2a/tests.rs` (`gateway_tasks_go_to_the_gateway_handler`, `the_gateway_reaches_agents_that_dont_take_direct_delegation`, `a_gateway_task_gets_a_provider_row_with_the_routing_reason`), `relay/tests.rs` (`tasks_reach_a_gateway_only_instance`) | Layer: api (two instances plus a model key) | Gap: none for the composer (`MemberProfileView.test.tsx`).
- **Needs relay:** yes
- **Status:** Shipped

### NW-09 — Sharing: What you offer (overlaps the gateway area)

- **What:** Each capability your agents serve sits in one of three columns: Offered, Needs your approval, or Hidden. The page also shows the "About this Nandi" text (the description and NANDI.md body without its private `<!-- -->` comments, `publicAbout`), rendered as markdown (headings and lists, never raw `##`; `OfferAbout`), max concurrent requests and rate limit, and "Edit as file" (opens Studio on the gateway policy). Saving rewrites the NANDI.md `expose`, `hide` and `requireApproval` lists.
- **Where:** Network → Sharing → What you offer. Code: `src/components/network/OfferView.tsx`, `offer-model.ts`.
- **Preconditions:** Local agents with `serves:`.
- **Steps:** 1. Open Network → Sharing → What you offer and click **Edit**. 2. Set one capability to "Hide" and one to "Needs approval". 3. Click **Save**.
- **Expected:** The toast "Saved what you offer" (`network.offer.saved`) lists the changes. The manifest is updated.
- **Edge / negative cases:**
  - With no capabilities, the page shows `network.offer.nothing`.
  - Policy entries that no local agent serves are kept.
- **Automation:** Covered by: `src/components/network/offer-model.test.ts` | Layer: unit | Gap: no UI test.
- **Needs relay:** no
- **Status:** Shipped

### NW-10 — Sharing: Incoming requests

- **What:** Lists tasks other Nandis sent you, refused ones included. Filters: All, Waiting on me, Running, Done, Refused. Refused rows say "refused: not trusted yet" or "refused". A row opens its Request page.
- **Where:** Network → Sharing → Incoming requests (`SharingView.tsx` `IncomingRequests`, `useIncomingTasks`).
- **Preconditions:** You have received at least one remote task.
- **Steps:** 1. Open the tab. 2. Filter Refused. 3. Click a row.
- **Expected:** Rows read "{capability} for {agent}" with "From {name} · {when}". The Request page opens.
- **Edge / negative cases:** The empty state reads "No one has asked your Nandi for anything yet."; when you are not reachable it shows a **Make this Nandi reachable** button that opens Connection.
- **Automation:** Covered by: none dedicated | Layer: ui-e2e | Gap: no test for IncomingRequests.
- **Needs relay:** yes
- **Status:** Shipped

### NW-11 — Sharing: Capabilities tab (overlaps the capability area)

- **Duplicate:** tested under CP-11. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### NW-12 — Reassign a task you are serving to a teammate

- **What:** A provider who is serving a delegated task can hand it to an online member who serves the same capability. The relay moves it (relay design step 13): it checks the chain (never back to the requester or anyone who had it), delivers the request resealed for the new provider, and tells the requester. The requester never resubmits, and the task keeps its ID. The provider's row becomes `Reassigned`, its run stops, and `task://reassigned` fires.
- **Where:** Agent task strip → provider state panel → **Reassign** menu, which is shown only when candidates exist and the task state is reassignable (`src/components/agent-console/ReassignMenu.tsx`, `ProviderStatePanel.tsx`). Code: `rpc/reassign.rs` (`reassign_task`, `list_reassign_candidates`), `services/a2a/reassign.rs`, `relay/client.rs` (`reassign`). HTTP: `POST /api/rpc/reassign_task {"newProviderInstance","reason"?}`, `POST /api/rpc/list_reassign_candidates`. Relay: `Reassign` / `ReassignResult`, nandi-relay `src/dispatch.rs`.
- **Preconditions:** Three instances (requester C, provider A, teammate B) on a relay. B serves the same capability, and C's task to A is Delivered, Acknowledged, Working or InputRequired.
- **Steps:** 1. C delegates to A. 2. On A, open Reassign and pick B. 3. Watch C and B. 4. Take C offline during step 2.
- **Expected:**
  - A sees the toast "Reassigned to {name}" (`tasks.provider.reassigned`); its row is Reassigned.
  - C's thread shows "Reassigned to Bob by Alice" (with A's reason) on the same task, which now names B as its provider; C gets a toast.
  - B runs the task as an ordinary task, and the result reaches C, even when C was offline during the move.
  - A request C sent to A as a person (an ask or an assignment to A's gateway) goes to B's gateway, so B's Nandi holds an assignment for B to approve like any other.
  - A `reassigned_tasks` row is written on both A and C.
- **Edge / negative cases:**
  - With no candidates, the menu is hidden.
  - Reassigning to the requester, to yourself, or to an instance that already had the task is refused (by the app for what it knows, by the relay for the whole chain).
  - A that restarted no longer holds the opened request: the relay asks C, the author, to seal it again, and B gets it then.
  - A manager's move goes through the relay's management API and waits for A's Nandi to reseal (RL-44).
  - A non-reassignable state is an error.
- **Automation:** Covered by: `a2a/tests.rs` (`a_provider_reassigns_a_task_and_the_requester_gets_the_result`, `peers_can_only_move_their_own_tasks`); `relay/sealed_tests.rs` (`a_reassigned_task_reaches_the_new_provider_signed_by_the_requester`, against a real relay); nandi-relay `tests/dispatch.rs` (`a_provider_hands_a_task_on_and_nobody_gets_it_twice`, `a_managers_move_waits_for_the_reseal`); `src/components/agent-console/TaskStatusPanel.test.tsx`, `src/components/views/DelegationThreadView.test.tsx`, `src/lib/agent-sync.waveef.test.ts` | Layer: api (three instances) | Gap: no direct ReassignMenu test.
- **Needs relay:** yes
- **Status:** Shipped

### NW-14 — Availability: busy or away until, and who covers

- **What:** A person sets whether they're around under Network › Sharing › What you offer › Availability (Available, Busy or Away; until which day; who covers, from the people on the relay), or by hand in their NANDI.md frontmatter (the gold copy):

  ```yaml
  availability:
    status: away # available, busy or away
    until: 2026-11-10 # a date, or a time like 2026-11-10T09:00:00+01:00
    delegate_to: bob-laptop-1a2b3c4d # who covers (their device ID)
  ```

  It goes out on their gateway card. People cards and the member profile show it small and calm: "Away until Tue, Nov 10 · Bob covers" (the name from the directory, else the ID; "You cover" when it's this Nandi, which isn't in its own directory). Once `until` passes it shows nothing. `find_people` results carry it too (RL-27).

- **Where:** `services/gateway/manifest.rs` (`AvailabilityYaml`, `parse_until`), `gateway/registration.rs` (card), `src/components/network/AvailabilityEditor.tsx`, `MemberAvailability.tsx`, `availability-model.ts` (`choiceFrom`, `availabilityOf`), `MemberCardCompact.tsx`, `MemberProfileView.tsx`.
- **Preconditions:** Two Nandis on one relay.
- **Steps:** 1. On B, add the block above to NANDI.md (with A's device ID as `delegate_to`). 2. On A, open People, then B's profile. 3. On B, set `until` to a minute from now and wait. 4. On B, write `status: vacation`. 5. On B, fix the file, then in What you offer › Availability choose Away, pick tomorrow, choose A under Who covers, and Save availability. 6. Choose Available and save.
- **Expected:** Step 2 shows "Away until … · A covers" on the card and the profile. After step 3's time it disappears. Step 4 makes NANDI.md invalid with the message naming `vacation` (the gateway keeps its last good card). Step 5 shows "Availability saved", writes `status: away`, tomorrow's date and A's ID to NANDI.md, and A sees "Away until … · You cover". Step 6 removes the block's `until` and `delegate_to`, and A sees nothing.
- **Edge / negative cases:** `status: available` drops `until` and `delegate_to`; an unreadable `until` or a `delegate_to` with spaces or `/` is an error that says what to fix; an unknown key under `availability:` is an error.
- **Automation:** Covered by: `gateway/manifest.rs` (`availability_is_read_checked_and_written_back`), `availability-model.test.ts` (including `choiceFrom and availabilityOf`), `NetworkView.test.tsx` (`says "You cover" when this Nandi covers (it is not in the list)`, `says when someone is away and who covers`), spec `directory.rs` (`availability_ends_at_its_until`) | Layer: unit + ui-e2e | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### NW-13 — Device (instance) ID

- **Duplicate:** tested under ST-14. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

## Relay

### RL-01 — Connection: Join a company relay, Change

- **What:**
  - With no relay, the card reads **Join a company relay**: the **Relay URL** (empty, with the example `wss://relay.example.com/relay` as its placeholder; prefilled only with the build's own relay, never a developer address), a **Join code** password field and **Paste invite**. Nothing connects or enrolls until you click **Join** (`join_relay`, RL-38). A joined relay shows a toast "Joined <host>. Connecting…"; anything else shows why under the form and changes nothing.
  - Once joined, the URL shows as text with **Change…**, **Test connection**, **Copy invite message** and **Disconnect**. The text never shows this instance's `?token=`; neither do invites, `NetworkSnapshot.relayUrl` or `RelayStatus.relayUrl` (the command, `GET /api/relay/status` and `relay://status` strip it: `RelayStatus::without_token`).
  - The URL goes to `~/.nandi/config.json` (`relay_url`), and the app reconnects on every launch.
- **Where:** Network → Connection (`src/components/views/ConnectionView.tsx` `RelayCard`, `components/network/JoinRelayFields.tsx`, `src/hooks/useRelay.ts` `useJoinRelay`, `useCopyInvite`, `DEFAULT_RELAY_URL`). Code: `commands/relay.rs` (`get_relay_status`, `join_relay`, `get_relay_invite`), `services/relay/join.rs`, `services/relay/mod.rs` (`resolve_url`). HTTP: `GET /api/relay/status`, `POST /api/relay/join {"url","joinCode"}`, `GET /api/relay/invite`. Event: `relay://status`.
- **Preconditions:** A relay is running (RL-07), with a join code for the secured case.
- **Steps:** 1. On a fresh install, open Network → Connection. 2. Enter `wss://<relay>/relay` and the join code (or Paste invite). 3. Click **Join**. 4. Restart the app. 5. Click **Change…**, edit the URL and click **Join**.
- **Expected:**
  - The toast reads "Joined <host>. Connecting…"; the join code field is gone.
  - The pill moves through Connecting… to Connected, and the summary reads "Last heartbeat {time} · N Nandis on this relay".
  - `config.json` has `relay_url` (no token), and after the restart the app connects automatically.
- **Edge / negative cases:**
  - An invalid URL (not ws:// or wss://) disables Join and marks the field `aria-invalid`.
  - A `?token=…` in the URL (operator-issued) joins as it is; the token goes to SQLite `relay_state`, never `config.json`.
  - `wss://` URLs reach the TLS handshake.
  - Wrong code, code needed, relay full (the relay's words verbatim), version mismatch, unreachable: one calm line each, nothing saved.
- **Automation:** Covered by: `ConnectionView.test.tsx` (join with code, full relay's words and no code on screen, paste invite), `src/hooks/useRelay.test.ts`; Rust `relay/join_tests.rs`, `relay/tests.rs` (`setting_the_url_connects_and_lists_online_agents`), `relay/client.rs` (`tokens_are_split_out_of_the_url`, `wss_reaches_the_tls_handshake_without_panicking`) | Layer: api + ui | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-02 — Test connection

- **What:** Pings the URL in the field with a WebSocket, with a 2 s probe timeout and a 5 s connect timeout. It shows the round trip in ms or the exact error.
- **Where:** Connection card, and the checklist's relay row. Code: `commands/relay.rs` (`test_relay_connection`). HTTP: `POST /api/relay/test {"url"}`.
- **Preconditions:** None.
- **Steps:** 1. Test a running relay. 2. Test a closed port. 3. Test a firewalled LAN host.
- **Expected:** The result is `network.connection.testOk` with the ms for the running relay, and `testFailed` with the error (refused or timeout) for the others.
- **Edge / negative cases:** Test is disabled for invalid URLs.
- **Automation:** Covered by: `relay/tests.rs` (`test_connection_answers_with_the_round_trip_or_the_exact_error`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-03 — Disconnect

- **What:** After a confirmation, clears the saved URL. The app disconnects and stays disconnected on the next launch. The instance token is kept for that relay, so joining it again reuses the seat (no second enrollment).
- **Where:** Connection card → Disconnect (ConfirmAction "Disconnect from the relay?"). The same call is `PUT /api/relay/url {"url":""}`.
- **Preconditions:** A relay is saved.
- **Steps:** 1. Click **Disconnect** and confirm with **Disconnect**. 2. Restart the app.
- **Expected:** The toast reads "Disconnected from the relay", `relay_url` is removed, and the Network page defaults to Connection.
- **Edge / negative cases:** Disconnect and Change are disabled while `NANDI_RELAY_URL` is set.
- **Automation:** Covered by: `relay/tests.rs` (`relay_urls_are_validated_and_cleared`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-04 — Relay URL precedence (`NANDI_RELAY_URL` > config.json > none)

- **What:** `NANDI_RELAY_URL` wins, then `relay_url` in `config.json` (with the instance token from `relay_state`). `NANDI_RELAY_URL` is not read. The Connection card says "Set by your environment; changes here won't apply." with **Change** and **Disconnect** disabled, and Details shows "Relay address set by: An environment variable (NANDI_RELAY_URL)". A `PUT /api/relay/url` still takes effect for the running app, but the environment variable wins again at the next launch. With neither set, the app stays disconnected.
- **Where:** `services/relay/mod.rs` (`resolve_url`, `RELAY_URL_ENV`), `RelayStatus.relayUrlSource`.
- **Preconditions:** None.
- **Steps:** 1. Launch with `NANDI_RELAY_URL=ws://127.0.0.1:8787/relay`. 2. Open Connection and Details.
- **Expected:** The app is connected without any saved config, and Details names the environment variable as the source (`relayUrlSource: "env"` in `GET /api/relay/status`).
- **Edge / negative cases:** A relay URL stored anywhere else (for example a `relay_state.relay_url` row) is ignored; with no env and no `config.json` value the app is disconnected.
- **Automation:** Covered by: `relay/tests.rs` (`the_relay_url_comes_from_the_env_then_config`, `without_a_relay_status_is_disconnected`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-05 — Connection status, reconnect and Details

- **What:**
  - The status pill shows Connected, Connecting…, Reconnecting…, Disconnected or Error, with the reason.
  - After a drop, the client reconnects with backoff of 1, 2, 4, 8 and 16 s, then every 30 s, and re-registers its cards.
  - Details (open or closed is remembered in localStorage `nandi.network.detailsOpen`) shows the device ID with Copy, the URL source, last connected, agents online, the reason, and a "Forward requests to trusted people" switch (the NANDI.md `federation.enabled` setting).
- **Where:** Connection → `ConnectionDetails`. Code: `services/relay/client.rs`.
- **Preconditions:** A relay is saved.
- **Steps:** 1. Stop the relay. 2. Watch the pill and the reason ("…; retrying in 8s"). 3. Restart the relay. 4. Open Details and toggle "Forward requests to trusted people".
- **Expected:** The pill goes Reconnecting, then Connected, and agents re-register. The switch persists to NANDI.md.
- **Edge / negative cases:** Details still renders when localStorage is blocked; it just starts closed.
- **Automation:** Covered by: `relay/client.rs` (`backoff_doubles_up_to_thirty_seconds`, `connecting_to_nothing_fails_but_keeps_the_url`), `relay/tests.rs` (`a_dropped_client_reconnects_and_re_registers`) | Layer: api | Gap: no UI test for Details.
- **Needs relay:** yes
- **Status:** Shipped

### RL-06 — "Make this Nandi reachable" checklist (7 rows)

- **What:** Shows every opt-in a remote request goes through, each row with one fix:
  1. Connected to a relay (Connect or Test).
  2. Listed (makes the gateway discoverable).
  3. Confirmed by the relay (heartbeat under 90 s, with a 30 s grace period).
  4. Agents that take requests by name ("Choose agents…" toggles `accepts_delegation`, or "None: only general requests").
  5. Who can send you work (trusted count and waiting trust requests → Review).
  6. Approvals waiting ("N requests are waiting for your approval", with Open Inbox).
  7. Visibility of others on the relay.

  The checklist is titled "Make this Nandi reachable" and the header shows "N of 7 done".

- **Where:** Network → Connection; rows 1–3 also appear on People's empty state. Code: `src/components/network/ReachabilityChecklist.tsx`, `reachability-model.ts`.
- **Preconditions:** None.
- **Steps:** 1. On a fresh install with the relay running, follow every fix button until all rows pass. 2. From a second instance, delegate to this one.
- **Expected:** All rows pass, and the first delegation succeeds without editing YAML or calling the API (the acceptance test of task 12).
- **Edge / negative cases:**
  - Rows 2–3 are "blocked" until row 1 passes.
  - When the relay never confirms (outdated relay, or a secret is required), row 3 shows `network.checklist.notConfirmed`.
- **Automation:** Covered by: `src/components/network/reachability-model.test.ts` | Layer: unit (the model); manual (end to end) | Gap: no component test.
- **Needs relay:** yes
- **Status:** Shipped

### RL-07 — Run a relay: `nandi-relay`

- **What:** A self-hosted WebSocket relay with presence and routing. It stores no content. It's a separate product and repo (`vaultdbai/nandi-relay`); the app doesn't run it (`nandi relay` is gone):
  - `nandi-relay [--host H] [--port N] [--tls-cert C --tls-key K] [--licence FILE] [--open]`, which defaults to `0.0.0.0:8787`.

  **It refuses to start without `NANDI_RELAY_SECRET` unless given `--open`** (exit code 2, with a message saying why). `-h` prints usage. It logs one line per event: `+ connection`, `~ re-registered`, `! bad message`, `- connection closed (reason=client_disconnect|heartbeat_timeout|protocol_error|replaced|revoked)`.

- **Where:** nandi-relay: `src/main.rs`, `src/standalone.rs`, `src/server.rs`. Helper scripts there: `scripts/test-relay-local.ps1`/`.sh` and `scripts/test-relay-lan.ps1`/`.sh`.
- **Preconditions:** A built binary (`cargo build --release` in nandi-relay). The release pipeline does not ship it yet.
- **Steps:** 1. Run `nandi-relay --host 0.0.0.0 --port 8787` with no secret. 2. Run it again with `--open`. 3. Connect two instances. 4. Kill one instance.
- **Expected:**
  - Step 1: "Refusing to start: NANDI_RELAY_SECRET isn't set…", exit code 2.
  - Step 2: the log shows `relay started on ws://0.0.0.0:8787/relay (auth: open)` and that auth is off (`--open`).
  - Steps 3–4: a `+ connection … agents=N` line per instance, then `- connection closed … reason=client_disconnect`.
- **Edge / negative cases:**
  - Unknown flags and bad port values are rejected with usage.
  - `--tls-cert` without `--tls-key` (or the other way round) is refused.
  - `nandi relay` and `nandi --relay` aren't commands of the app: they fail with "Unknown option".
  - A port that is already taken fails to bind.
  - `--host 127.0.0.1` accepts local clients only.
- **Automation:** Covered by: nandi-relay `src/standalone.rs` (`defaults_to_every_interface_on_8787`, `parses_host_and_port`, `rejects_bad_values_and_unknown_flags`, `parses_help`, `refuses_to_start_open_without_the_flag`) | Layer: api | Gap: no test of the binary's log output.
- **Needs relay:** yes
- **Status:** Shipped

### RL-08 — Relay HTTP endpoints: `/health`, `/instances`

- **What:**
  - `GET /health` returns `{status, instances_connected, uptime_seconds, version, auth}`.
  - `GET /instances` lists each instance's ID, peer address, agent cards, connected_at, last_heartbeat and seconds_since_heartbeat.
  - The app shows the instances through `get_relay_instances`.
- **Where:** nandi-relay `src/server.rs`. App side: `commands/relay.rs` (`get_relay_instances`, `list_online_agents`). HTTP: `GET /api/relay/instances`, `GET /api/relay/agents?capability=`.
- **Preconditions:** A relay is running.
- **Steps:** 1. `curl http://<relay>:8787/health`. 2. `curl …/instances`. 3. `GET /api/relay/agents`.
- **Expected:**
  - `"auth":"open"` when no secret is set.
  - The instances list matches the connected apps.
  - Online agents leave out the `<instance>/gateway` route cards.
- **Edge / negative cases:** While offline, agent lists come from the last known agents.
- **Automation:** Covered by: `relay/tests.rs` (`the_app_connects_to_an_open_relay_and_lists_its_instances`, `offline_lists_come_from_the_last_known_agents`), nandi-relay `tests/relay.rs` (`an_open_relay_routes_between_two_instances_and_reports_them`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-09 — Relay token auth (`NANDI_RELAY_SECRET`)

- **What:** When the relay has `NANDI_RELAY_SECRET` set, registering needs an instance token. The operator gets it with `GET /relay/token?instanceId=<id>` and `Authorization: Bearer <secret>`. The user adds it to the URL as `?token=…`.
- **Where:** nandi-relay `src/server.rs` (`SECRET_ENV`, the `/relay/token` route). Client: `relay/client.rs`.
- **Preconditions:** A relay started with the secret set.
- **Steps:** 1. Connect without a token. 2. Fetch a token for the instance ID. 3. Save the URL with `?token=`.
- **Expected:** Without a token the relay logs `! registration refused` and checklist row 3 is not confirmed. With the token, the instance registers.
- **Edge / negative cases:**
  - A token issued for another instance is refused.
  - An empty secret is refused.
  - `/relay/token` without the secret is refused.
- **Automation:** Covered by: nandi-relay `src/server.rs` (`tokens_verify_only_for_their_instance_and_secret`, `an_empty_secret_is_refused`, `the_token_endpoint_needs_the_secret`), `relay/tests.rs` (`registration_needs_a_valid_token`) | Layer: api | Gap: there is no UX for issuing tokens.
- **Needs relay:** yes
- **Status:** Partial (the token flow works, but there is no end-to-end UX; relays are open by default)

### RL-10 — Presence: heartbeats, eviction, capability presence

- **What:**
  - Instances heartbeat every 30 s, and the relay evicts an instance after 90 s of silence.
  - Registration carries providers and tools (`relay::presence::publish`), and `UpdatePresence` sends changes.
  - Presence never says more than the cards (`only_shown`): a provider is sent only for a capability the gateway card exposes (`relay.enabled`, `expose`, `hide`) or an agent card serves. A capability whose definition is `private` (the default) is never sent. The tools sent are only the published providers' tools, not every enabled tool.
  - Cards that name another instance are dropped.
  - Turning on `accepts_delegation` re-registers the cards within about 500 ms.
  - `query_capability_presence` lists who serves a capability (`*` for all).
- **Where:** `services/relay/presence.rs`, `client.rs`, nandi-relay `src/server.rs`, `rpc/relay_capabilities.rs` (`query_capability_presence`).
- **Preconditions:** A relay and two instances.
- **Steps:** 1. Suspend instance B for more than 90 s. 2. Check `/instances`. 3. On A, turn on an agent's `accepts_delegation` and watch the relay. 4. On A, give a capability `visibility: private` and turn `relay.enabled` off; on B, run `query_capability_presence "*"`.
- **Expected:**
  - B is evicted (`reason=heartbeat_timeout`) and re-registers when it resumes.
  - The relay log shows `~ re-registered … agents=N+1` without a restart.
  - (4) B sees only A's delegating agents' non-private capabilities and those agents' tools; nothing of A's private capability, workflows or other tools.
- **Edge / negative cases:**
  - Queries sent before registration wait for it.
- **Automation:** Covered by: `relay/tests.rs` (`silent_instances_are_evicted`, `cards_of_other_instances_are_dropped`, `agents_turned_on_while_connected_reach_the_relay`, `queries_sent_before_registration_wait_for_it`), `relay/presence.rs` (`registration_carries_presence_and_updates_replace_it`, `groups_local_providers_by_capability`, `nothing_is_published_that_no_card_shows`, `an_agent_card_publishes_only_its_own_capabilities_and_tools`, `the_gateway_publishes_every_provider_of_what_it_exposes`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-11 — Two instances on one machine (`NANDI_HOME`)

- **What:** `NANDI_HOME` replaces `~/.nandi`: the state DB, repo, config, logs, web token and instance ID. With it set, the single-instance lock is skipped, so a second window can open.
- **Where:** nandi-relay `docs/relay-testing.md`, `cli.rs` (Environment help).
- **Preconditions:** A debug or release build and a relay.
- **Steps:** 1. Start A with `NANDI_HOME=~/.nandi-a NANDI_RELAY_URL=…`. 2. Once A's window is open, start B from `src-tauri` with `NANDI_HOME=~/.nandi-b`.
- **Expected:** Both windows run with different instance IDs and each sees the other on People. Without `NANDI_HOME`, the second launch just focuses the first window.
- **Edge / negative cases:**
  - Both instances share the OS keyring and the webview storage.
  - Instances launched at the same moment can hang creating their WebView2 windows.
  - The companion hotkey belongs to the first instance only.
- **Automation:** Covered by: none (process level) | Layer: manual (or api with `--headless --port`) | Gap: no scripted two-instance end-to-end test in CI.
- **Needs relay:** no
- **Status:** Shipped

## Delegation (agent to agent)

### RL-12 — Agent delegation settings

- **What:** Per agent:
  - "Can call other agents" adds the `delegate_to_agent` tool, and each call asks you first.
  - "Accepts tasks from other agents".
  - "Tasks at once" (`max_concurrent`).
  - Per-capability exceptions: Always accept, Never accept, or Use the setting above.
  - Lists of "Agents it can call" and "Agents that can call it".

  Settings are written to skill.md through the repos layer, never to SQLite.

- **Where:** Agent → Settings tab → Sharing section (`src/components/agent-console/DelegationPanel.tsx` inside `src/components/agents/AgentSettingsTab.tsx`). Code: `commands/delegation.rs` (`get_agent_delegation`, `set_agent_delegation`), `services/a2a/settings.rs`. HTTP: `GET`/`PUT /api/agents/{agent_id}/delegation`.
- **Preconditions:** An agent exists.
- **Steps:** 1. Turn on both switches. 2. Set Tasks at once to 2. 3. Add an exception `summarize: Never accept`. 4. Open skill.md.
- **Expected:** skill.md has `delegation_outbound: true`, `accepts_delegation: true`, `max_delegation_concurrent: 2` and `delegation_overrides: {summarize: deny}`, and the gold copy commit is made.
- **Edge / negative cases:**
  - Invalid settings are rejected; on failure the toast is `delegation.panel.saveFailed`.
  - An explicit `allowed-tools` list follows the outbound toggle.
  - Both are off by default.
- **Automation:** Covered by: `src/components/agent-console/DelegationPanel.test.tsx`, `src/hooks/useDelegation.test.ts`; Rust `a2a/settings.rs` (`defaults_are_off`, `settings_round_trip_through_skill_md`, `an_explicit_tool_list_follows_the_outbound_toggle`, `invalid_settings_are_rejected`), `a2a/tests.rs` (`settings_are_written_to_skill_md`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-13 — `delegate_to_agent` tool, with approval

- **What:** An agent with outbound delegation can call `delegate_to_agent(capability, input, target?)`. Each call goes through the normal approval path (an approval row and `agent://approval_request`), and the provider is picked only after you approve. Declaring `requires_approval` without the tool skips approval.
- **Where:** `services/a2a/tool.rs`, the orchestrator approval path; Approve/Deny in the chat or Inbox.
- **Preconditions:** An agent with "Can call other agents" on, another agent that serves the capability, and a model key.
- **Steps:** 1. Ask the agent to have someone summarize a text. 2. Approve the tool call.
- **Expected:** The task runs on the provider and the agent receives the result. A `delegations` row and a `~/.nandi/local/logs/delegations.log` line are written, and the `delegation://task_started` and `delegation://task_completed` events fire.
- **Edge / negative cases:** If you deny, the task never starts.
- **Automation:** Covered by: `a2a/tests.rs` (`the_delegation_tool_waits_for_approval`, `declaring_approvals_without_the_tool_skips_approval`, `local_delegation_works_without_a_relay`) | Layer: api (mock LLM) / manual (real model) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-14 — Delegate a task: ranking, fallback, timeout, failover at the relay

- **What:**
  - `delegate_task(capability, input, target?)` picks providers: a named card alone (never swapped; a person away may be covered by their own delegate), a pinned one the relay has free, then your own (they run here, no relay), else the relay's plan (`RouteQuery` → `RoutePlan`: the relay ranks, with your pin as preferred and the members you paused or blocked excluded). Without a relay, a capability only others serve answers `RelayUnavailable`.
  - The delegation thread shows "Why {name}" with the relay's reasons (`route` message).
  - Your own providers are tried in turn (up to 3); each passed-on attempt is kept as `<id>-attempt<n>`.
  - A request to someone else goes to the relay sealed for every candidate of its plan (`SubmitTask`). The relay gives it to the next one when the first refuses it, doesn't pick it up within 90 s, or (work for an agent) takes over 3× its ETA, with you offline or not. The thread says "Passed to {name}" with why (`failed_over` message), and the task keeps its ID. A named person and an assignment get no one else.
  - A remote request is waited on live for 600 s (`local_settings.delegation_timeout_secs`). Then it stays open until its due date (`input.due`, else 2 days for `ask-me`, 1 day otherwise) and the tool says `status: waiting`; a late answer is kept. Past its due date the relay ends it (abandoned if nobody took it up, else timed out) for both sides.
- **Where:** `commands/delegation.rs` (`delegate_task`, `list_available_providers`), `services/a2a/delegator.rs`, `task.rs` (`on_event`), `task_state.rs` (`hand_on`), `queue.rs`, `resume.rs`; nandi-relay `src/dispatch.rs`. HTTP: `POST /api/delegation/tasks`, `GET /api/delegation/providers?capability=`, `POST /api/rpc/get_eta_estimate`. Events: `delegation://*`, `task://state_changed`, `task://timed_out`, `task://circuit_opened`.
- **Preconditions:** At least two providers for the capability on other Nandis, one of which refuses.
- **Steps:** 1. Delegate with no target. 2. Make the first pick reject (trust or closed). 3. Separately, delegate to an offline member, then bring them online.
- **Expected:**
  - The rejected pick is passed to the next provider by the relay; the thread says "Passed to {name}".
  - After 3 failures in a row, that provider's circuit opens for 5 minutes (at the relay).
  - The offline member gets the task when they come online (RL-43).
- **Edge / negative cases:**
  - A named target is never swapped.
  - A target the relay doesn't know is refused as the answer to that send.
  - A task waiting for a clarification does not time out on its ETA.
  - **Retry** on a failed request makes a new attempt from a fresh plan without anyone it went to.
- **Automation:** Covered by: `a2a/tests.rs` (`one_instance_delegates_to_another_through_the_relay`, `a_rejected_pick_falls_back_to_the_next_provider`, `a_remote_request_without_an_answer_stays_open_until_it_is_due`, `a_delegation_to_an_offline_provider_waits_at_the_relay`, `a_restart_resumes_in_flight_tasks`, `a_retry_creates_the_next_attempt_elsewhere`, `eta_estimates_come_from_history`), `a2a/eta.rs`; nandi-relay `tests/dispatch.rs` (`a_refusal_goes_to_the_next_candidate_without_the_requester`, `nobody_picking_it_up_fails_over_then_the_due_date_ends_it`, `a_declined_assignment_and_the_last_refusal_reach_the_requester`), `src/dispatch.rs` (the rules) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-15 — Inbound admission (provider side)

- **What:** A received task is rejected, with a typed reason the requester sees, when any of these holds:
  - the agent doesn't accept delegation;
  - the agent doesn't serve the capability;
  - the agent is at `max_delegation_concurrent`;
  - the requester is denied, paused or blocked;
  - the policy is Ask and the requester is not a trusted peer.

  Accepted tasks run as a turn in the agent's own conversation, and the result is sent back. Gateway-routed tasks are taken on the gateway's word.

- **Where:** `services/a2a/executor.rs`, `a2a/task.rs` (`ReasonKind`, `TaskStage`).
- **Preconditions:** Two instances.
- **Steps:** 1. Call an agent with `accepts_delegation` off. 2. Fill its concurrency with tasks and call again.
- **Expected:** Rejected tasks carry typed reasons, and the requester's Request page shows a blocker with a fix button. Each accepted task gets its own conversation.
- **Edge / negative cases:** A peer can only cancel or move its own tasks.
- **Automation:** Covered by: `a2a/tests.rs` (`agents_that_dont_accept_delegation_reject_tasks`, `each_delegated_task_runs_in_its_own_conversation`, `a_cross_instance_delegation_walks_the_lifecycle_on_both_sides`, `status_queries_and_cancels_reach_the_provider`, `a_capability_needing_approval_waits_for_the_owner`, `a_busy_gateway_queues_the_request_until_a_slot_frees`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-16 — Delegation thread view

- **What:** One delegated task as a thread: delegated, state changes, progress (with %), status queries and responses with ETA, clarification requests and answers, notes, "Passed to X" when the relay failed it over, and "Reassigned to X by Y".
  - For a provider the relay picked, "Why {name}" lists the relay's reasons in your language ("Alice is away; Bob covers · Bob 3/3 busy (about 20 min wait)").
  - It shows a queued-offline banner and a partial-result line.
  - The side panel shows source, attempt, priority, created, ETA, deadline, the clarification policy summary, Rerun, Details, previous attempt, provider and provider profile.
  - **Open as request** opens the Request page.
- **Where:** Opened from Ask, Call dialog, the provider or task panels, and Request banners (`{kind:'delegation-thread', taskId}`). Code: `src/components/views/DelegationThreadView.tsx`. HTTP: `POST /api/rpc/get_task`, `/events`, `/messages`, `/status`, `POST /api/rpc/cancel_task`.
- **Preconditions:** A delegated task exists.
- **Steps:** 1. Open a thread for a completed remote task. 2. Open one for a reassigned task. 3. Open one for a task to an offline member.
- **Expected:** The entries render in order. A reassigned task stays one thread (same ID), now naming the new provider. The offline banner says "{name} is offline. The relay keeps the request and gives it to them when they're back."
- **Edge / negative cases:** An unknown task ID shows `delegationThread.loadFailed`.
- **Automation:** Covered by: `src/components/views/DelegationThreadView.test.tsx` | Layer: ui-e2e | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-17 — Clarifications between agents (respond, escalate, policy)

- **What:** A provider can ask the requester a question with the `ask_requester` tool. The requester's clarification policy routes the question by topic:
  - auto-answered by the requesting agent;
  - escalated to the human (an approval);
  - or failing the task: past max rounds, when the policy forbids questions, or on timeout.

  The question shows as a card in the thread, and you can answer it, take it over yourself ("Ask me") or let the agent answer. The policy editor sets allow, max rounds, auto topics, escalate topics and the timeout.

- **Where:** Thread `ClarificationCard`; Agent → Settings → Sharing → clarification policy (`src/components/agents/AgentDelegationTab.tsx`, `ClarificationPolicyEditor.tsx`). Code: `commands/delegation_clarification.rs` (`respond_to_clarification`, `escalate_clarification`, `get_pending_clarifications`, `get/set_clarification_policy`). HTTP: `POST /api/delegation/{task_id}/clarify`, `POST /api/delegation/{task_id}/escalate/{question_id}`, `GET /api/delegation/pending-clarifications`, `GET`/`PUT /api/agents/{agent_id}/clarification-policy`. Events: `task://clarification_requested`, `task://clarification_answered`.
- **Preconditions:** Two agents (local or cross-instance) and a provider prompt that triggers a question.
- **Steps:** 1. Set the escalate topics to `budget`. 2. Delegate a task whose provider asks about the budget. 3. Answer in the card.
- **Expected:** The task goes to `inputRequired`, then resumes after your answer and completes. The saved policy is bounded, and its topics are trimmed and de-duplicated.
- **Edge / negative cases:**
  - Denying cancels the task on both sides.
  - No answer by the deadline times the task out.
  - An offline requester ends the wait.
- **Automation:** Covered by: `services/a2a/clarification/tests.rs` (19 tests, e.g. `questions_route_by_policy_with_escalation_as_the_default`, `two_delegators_exchange_two_rounds_and_complete`, `denying_cancels_the_task_on_both_sides`), `src/components/agent-console/ClarificationCards.test.tsx`, `ClarificationPolicyEditor.test.tsx` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-18 — Delegation history and audit log

- **What:** Every delegation, in both directions, is recorded in `delegations` (one row per task and side) and in `~/.nandi/local/logs/delegations.log` (JSONL: who, capability, status, duration, error; never the input or output). It shows in Home → All activity next to runs. Each row opens its Request.
- **Where:** Home → All activity (`{kind:'home-activity'}`; `src/components/home/activity-model.ts` `delegationRow`). Code: `commands/delegation.rs` (`list_delegation_history`), `services/a2a/history.rs`. HTTP: `GET /api/delegation/history?limit=`.
- **Preconditions:** At least one delegation.
- **Steps:** 1. Make a delegation. 2. Open Home → All activity. 3. Read `delegations.log`.
- **Expected:** The row shows direction, the other party, status and duration. The log has no payload.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `a2a/history.rs` (`rows_round_trip_newest_first`, `the_audit_log_leaves_out_input_and_output`), `src/components/home/activity-model.test.ts` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-27 — Find who is responsible for a topic (`find_people`)

- **What:** A read-only built-in tool for agents whose skill.md lists `find_people` in `allowed-tools` (the first-party Chief of Staff agent does).
  - The relay searches and ranks (`FindPeople` → `PeopleMatches`), over its member directory (online or not) and its members' online agents: `responsibilities` from NANDI.md (GW-01), description, the capabilities their agents serve, their agents' cards, and the NANDI.md body when they share it.
  - Weights: responsibilities ×5, capabilities ×3, profile and agent cards ×2, body ×1. "SOC 1", "soc1" and "SOC-1" match alike, and an empty topic lists everyone.
  - Someone busy or away carries `availability: {status, until, covered_by}` (NW-14). `covered_by` is the person's name (from the directory or the people found), or "you" when the user asking covers; never a bare instance ID, which a model reads as one of the person's agents.
  - Each result has what matched, online or last seen, `ask: {target: <instance>/gateway, capability: ask-me|gateway}`, and the person's agents, each with its `target` and what it `serves` (capability and description), so a task can go straight to, for example, someone's PR Reviewer (`review-pr`). With a topic these are the agents that match it; listing everyone, all of them. `agents_online` counts all the person's agents online, so a person matched on their responsibilities isn't described as having no agents. The result also counts people without stated responsibilities.
  - When the relay isn't connected, it answers `RelayUnavailable` ("not connected"): nothing is matched from a cached copy.
  - It never needs approval and is never offered to an agent that doesn't list it.
- **Where:** `src-tauri/src/services/a2a/people.rs` (`find_people`, `person_json`, `offered`), `relay/client.rs` (`find_people`), relay repo `src/directory.rs` (`find_people`, the ranking), `services/orchestrator.rs` (offered after `delegate_to_agent`, answered on the host in `run_call`), `services/tools/builtin.rs` (catalog entry, tags `people`, `network`). Docs `docs/developer/a2a-delegation.md` › Finding People and Tracking Requests.
- **Preconditions:** Two or three Nandis on one relay, each with the gateway on and `responsibilities:` in NANDI.md (e.g. `[SOX audit, Risk]` on one and `[SOC 1 evidence]` on another); Chief of Staff installed on the asking one.
- **Steps:** 1. Ask Chief of Staff "Who is responsible for the SOX audit?". 2. "Who handles SOC1?". 3. "Who's on my network?". 4. With PR Reviewer (dev-pack 0.3.0, serves `review-pr`) installed on two of the others and `vaultdb` in one's responsibilities: "Who can review my pull request on vaultdb?", then "Ask whoever is online". 5. Stop the relay and ask again. 6. Remove `find_people` from Chief of Staff's `allowed-tools` and ask again.
- **Expected:**
  - Step 1 lists the person with `SOX audit` first and says why they matched and whether they're online.
  - Step 2 finds the `SOC 1 evidence` owner.
  - Step 3 lists everyone, online first, each with all their agents (Network's agent count and the answer agree).
  - Step 4 lists the vaultdb person first with their PR Reviewer, then, after approval, sends the PR to that agent (`review-pr`), whose review comes back as the answer.
  - In step 5 the tool answers "not connected" and the agent says it can't look people up until the relay is back.
  - In step 6 the tool isn't offered.
- **Edge / negative cases:** nobody matches: an empty list and an honest "nobody has said so"; people without stated responsibilities are counted so the answer can say the list may be incomplete; a person with the gateway on but not discoverable still matches on responsibilities, not on body text.
- **Automation:** Covered by: relay repo `src/directory.rs` tests (`stated_responsibilities_rank_above_mentions_in_a_profile`, `spacing_and_punctuation_dont_hide_a_match`, `an_empty_topic_lists_everyone_online_first`, `agent_cards_count_and_say_how_to_ask`, `reviewers_who_work_on_the_repo_come_first_with_the_agent_to_call`, `the_limit_counts_and_availability_is_as_of_now`) and `tests/directory.rs` (`people_are_found_by_what_their_cards_say`); app `a2a/people.rs` (`a_match_says_how_to_ask_and_what_their_agents_serve`, `someone_away_says_until_when_and_who_covers`, `only_agents_that_list_them_get_the_people_tools`), `relay/tests.rs` (`the_directory_keeps_members_who_left_and_people_are_found_at_the_relay`), `workflows.rs` (`an_ask_by_topic_finds_the_owner_and_asks_first`) | Layer: unit + api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-37 — "Update Nandi or the relay" (protocol version mismatch)

- **What:** Nandi and the relay each speak a protocol version (`PROTOCOL_VERSION` in `nandi-protocol`, now 2). Registration carries Nandi's; the relay's `/health` reports its own as `protocol_version`. A relay on another version refuses the registration with the error code `version_mismatch`. Nandi then shows Error on Connection with "Update Nandi or the relay: they speak different versions, so people features are off until they match." and treats person-to-person actions as not connected (RL-36). No other screen nags. A new connection (the relay restarted after an update) registers again on its own.
- **Where:** Connection (`ConnectionView.tsx`, `data-testid="relay-error"`). Code: `services/relay/client.rs` (`refused`, `version_mismatch`), `RelayStatus.versionMismatch`; relay repo `src/server.rs` (`RegisterInstance`).
- **Preconditions:** A relay built from another protocol version than the app (or the relay's tests).
- **Steps:** 1. Point Nandi at the older relay. 2. Open Network › Connection. 3. Open People and try Ask. 4. Update the relay and restart it.
- **Expected:** Step 2 shows Error with "Update Nandi or the relay"; step 3 shows "Connect to your company's relay"; after step 4 Nandi connects again without a restart.
- **Edge / negative cases:** A bad token still shows "Can't reach the relay…" (the usual words), not the version sentence. Heartbeats stop while refused.
- **Automation:** Covered by: relay repo `tests/directory.rs` (`a_registration_with_another_protocol_version_is_refused`), app `relay/tests.rs` (`a_relay_on_another_protocol_version_reads_as_not_connected`), `ConnectionView.test.tsx`, spec `message.rs` (`messages_are_tagged_snake_case_with_camel_case_fields`) | Layer: unit + api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-28 — Open requests both ways (`my_requests`)

- **What:** A read-only built-in tool, offered like RL-27. It lists requests between the user and other people from the `tasks` log: `asked_by_me` (with `to`) and `asked_of_me` (with `from`). It also lists requests the gateway holds for the user's approval as "waiting for your approval", first.
  - Arguments: `direction` (`asked_by_me`, `asked_of_me` or `all`), `open_only` (default true, which hides finished tasks) and `limit`.
  - Each entry has the request text, status, created and updated times, the due date, and the answer or error when there is one.
- **Where:** `src-tauri/src/services/a2a/people.rs` (`my_requests`, `requests_from`, `pending_holds`).
- **Preconditions:** RL-27's setup; one request sent from Chief of Staff (approved) and one received; NANDI.md `policy.require_approval: [assign-me]` (the default, GW-01) on the receiving side.
- **Steps:** 1. Ask Chief of Staff "Ask <person> for the SOC 1 evidence" and approve. 2. From the other Nandi, have Chief of Staff assign you work ("Assign <you>: rotate the keys by Friday") and approve there. 3. On yours, ask "What am I waiting on?" and "What did people ask me?". 4. "Show finished ones too."
- **Expected:**
  - Step 3 lists the first request under asked-by-me with its status, and the held assignment first under asked-of-me as "waiting for your approval".
  - Step 4 includes completed requests with their answers.
- **Edge / negative cases:** an unknown `direction` is refused; with no requests the list is empty.
- **Automation:** Covered by: `a2a/people.rs` (`requests_split_by_direction_and_hide_finished_ones`, `only_agents_that_list_them_get_the_people_tools`) | Layer: unit + manual | Gap: no end-to-end test across two instances.
- **Needs relay:** no
- **Status:** Shipped

## Git remote for the agents repo (overlaps settings/backup)

### RL-19 — Configure the git remote, credential and test

- **Duplicate:** tested under DT-18, DT-19, DT-21. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RL-20 — Push, pull, conflicts, branches (main and master protected)

- **Duplicate:** tested under DT-22, DT-23, DT-24, DT-25, DT-26. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

## Team-shared LLM setup

### RL-21 — `.nandi/llm.yaml` shared LLM config

- **Duplicate:** tested under MD-18. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

## HTTP/WebSocket server and web access

### RL-22 — Web server launch modes (`--server`, `--port`, `--headless`)

- **Duplicate:** tested under PL-02, PL-03. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RL-23 — Web auth token

- **What:**
  - Every request needs the token from `~/.nandi/web_token`: 64 hex characters, created on first launch, mode 0600 on Unix.
  - Scripts send `Authorization: Bearer <token>`. Browsers use the `nandi_token` cookie (`HttpOnly; SameSite=Strict`).
  - On a page load, `?token=` is exchanged for that cookie with a redirect that strips the token from the URL. For `/api/*` and `/ws`, `?token=` is accepted directly.
  - Comparison is constant-time.
  - Webhook and device trigger receivers skip the token (overlap with Triggers).
- **Where:** `src-tauri/src/http/auth.rs`.
- **Preconditions:** The server is running.
- **Steps:** 1. `curl /api/agents` with no token. 2. Repeat with the Bearer token. 3. Open `/?token=<t>` in a browser. 4. Open `/` without a cookie.
- **Expected:**
  - Step 1 gets `401 {"error":"unauthorized"}`. Step 2 gets 200.
  - Step 3 gets a 303 to `/` with Set-Cookie, and later navigation works.
  - Step 4 gets 401 with the plain-text message pointing at `~/.nandi/web_token`.
- **Edge / negative cases:**
  - A token that differs by one character is refused.
  - Other query parameters are kept when the token is stripped.
  - A token file shorter than 32 characters is regenerated.
- **Automation:** Covered by: `http/auth.rs` (`compares_tokens_exactly`, `strips_only_the_token_from_the_query`, `only_trigger_receivers_skip_the_token`, `creates_and_reuses_the_token_file`), `http/mod.rs` (`rejects_requests_without_a_valid_token`, `exchanges_a_query_token_for_a_cookie_and_strips_it`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RL-29 — A build's own relay, offered and joined only when the user agrees

- **What:** Release builds can carry a default relay, offered as "Join the Nandi network", without shipping the relay's secret.
  - **Build time:** `NANDI_DEFAULT_RELAY_URL` (a GitHub variable) and `NANDI_RELAY_JOIN_CODE` (a GitHub secret), read with `option_env!` by `BuiltInRelay::of_this_build` in `services/relay/mod.rs` and handed to the core in `Config::built_in_relay`. Set in `.github/workflows/release.yml`.
  - **Only an offer:** `NANDI_RELAY_URL`, else the saved `relay_url`, else nothing. The build's relay is never dialled and nothing enrolls at startup, so an install that says Not now (or never opens onboarding) takes no seat. `RelayStatus.builtInUrl` names it; once joined, `relayUrlSource: "builtIn"`, shown as "This build of Nandi (its own relay)".
  - **Joining:** onboarding's Continue on "Join the Nandi network" (or Join on Connection with that URL) calls `join_relay` with no code; the core uses the build's code, posts `/relay/enroll` once and keeps the token. Installs that joined reconnect at startup with it.
  - **Relay:** with `NANDI_RELAY_SECRET` and `NANDI_RELAY_JOIN_CODE`, `POST /relay/enroll {"instanceId","joinCode"}` returns `{instanceId, token}`. Errors: 401 wrong code, 400 bad ID, 402 no free seat, 409 already enrolled, 404 no enrollment on this relay. Enrolled IDs persist in `relay.db` under `--data-dir`.
- **Where:** `services/relay/mod.rs` (`BuiltInRelay`, `resolve_url_in`, `save_url`), `services/relay/join.rs` (`join_relay`, `enroll`, `enroll_endpoint`), nandi-relay `src/server.rs` (`enroll`, `with_join_code`), `docs/developer/relay-defaults.md`.
- **Preconditions:** A relay run with `NANDI_RELAY_SECRET=… NANDI_RELAY_JOIN_CODE=… nandi-relay`, and an app built with `NANDI_DEFAULT_RELAY_URL=ws(s)://that-relay/relay NANDI_RELAY_JOIN_CODE=…`.
- **Steps:**
  1. Start the built app with a fresh home and wait a minute.
  2. In onboarding, choose Not now; check the relay's `/health`.
  3. Open Network › Connection, enter the build's relay URL (no code) and Join; or in a second fresh home choose "Join the Nandi network" and Continue.
  4. Look at Network › Connection › Details. Restart the app.
- **Expected:**
  - Steps 1 and 2: no `/relay/enroll`, `/health` users unchanged, Connection says Disconnected.
  - Step 3: one enrollment, connected and registered.
  - Step 4: "Relay address set by: This build of Nandi"; after the restart it reconnects with its token, no second enrollment.
- **Edge / negative cases:**
  - A second install claiming the same instance ID gets 409 ("This Nandi joined … before").
  - `NANDI_RELAY_URL` still wins over everything.
  - The build's relay unreachable: joining says so; the app works the same (agents, workflows, packs, approvals), and person-to-person actions show "Connect to your company's relay" (RL-36).
- **Automation:** Covered by: `relay/join_tests.rs` (`a_fresh_install_with_a_build_relay_enrolls_only_when_it_joins`), `relay/tests.rs` (`a_build_s_relay_is_only_offered_until_the_user_joins_it`, `a_token_is_kept_for_its_relay_across_a_disconnect`, `enrolling_goes_next_to_the_relay_path`, `instances_enroll_once_with_the_join_code`), `offline_suite.rs` (`an_unreachable_default_relay_changes_nothing_local`: not configured, and joining it answers `unreachable`) | Layer: unit + api | Gap: no test that builds with the variables set.
- **Needs relay:** yes
- **Status:** Shipped

### RL-38 — Join a company relay with a join code, and invite a teammate

- **What:** A public installer joins a secured company relay from the UI: the relay's address and the join code the operator gives out (Connection, RL-01; onboarding, ON-09). `join_relay` checks `/health` (reachable, a Nandi relay, same protocol version), then enrolls once and stores the token as the build's flow does (`relay_state`). Outcomes (`JoinOutcome`): `joined`, `invalidUrl`, `unreachable`, `versionMismatch`, `codeNeeded`, `wrongCode`, `relayFull` (the relay's message shown verbatim), `alreadyEnrolled`, `removed`, `noEnrollment`, `signInNeeded` (a public relay: join it by signing in, RL-45), `refused`. An open relay joins without a code. The typed code is kept in the keychain (`nandi.relay.join_code`) only for **Copy invite message**, whose invite line is `<url>#join=<code>`; **Paste invite** (or pasting the message into the address) fills both fields. The code and the token are never shown or logged after entry.
- **Where:** `services/relay/join.rs`, `commands/relay.rs` (`join_relay`, `get_relay_invite`), `http/relay.rs` (`POST /relay/join`, `GET /relay/invite`), `components/network/JoinRelayFields.tsx`, `join-model.ts`, `relay-invite.ts`, `hooks/useRelay.ts` (`useJoinRelay`, `useCopyInvite`).
- **Preconditions:** A relay with `NANDI_RELAY_SECRET` and `NANDI_RELAY_JOIN_CODE`; a public build (no default relay).
- **Steps:** 1. Join with a wrong code. 2. Join with the right code. 3. Copy invite message, paste it into a second fresh install's address field, Join. 4. On a relay at its 20-user limit, Join. 5. Disconnect and Join again.
- **Expected:** 1. "That join code wasn't accepted…", nothing saved. 2. Joined and connected; the code field clears. 3. The message holds the address and `#join=<code>`, never the token; the second install fills both fields and joins. 4. "<host> has no free seat. The relay says:" and the relay's free-limit message. 5. Joined again with no second enrollment.
- **Edge / negative cases:** relay unreachable ("Couldn't reach a relay at …" with the detail); relay on another protocol version ("… speaks relay version …"); an ID enrolled before whose token is lost ("This Nandi joined … before"); the build's own code never goes into an invite.
- **Automation:** Covered by: Rust `relay/join_tests.rs` (9 tests), vitest `relay-invite.test.ts`, `join-model.test.ts`, `ConnectionView.test.tsx`, `OnboardingView.test.tsx`, `NetworkView.test.tsx` | Layer: api + ui | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-30 — Revoking a device on the relay (a leaver)

- **What:** The relay's operator turns an instance away at once and for good.
  - `POST /relay/revoke {"instanceId"}` with `Authorization: Bearer <secret>` closes that instance's connection (`reason=revoked`) and refuses its token from then on: registration, tasks and results.
  - `GET /relay/revoked` lists revoked IDs. `POST /relay/reinstate {"instanceId"}` lets one back in.
  - Revoked IDs persist in `relay.db` under `--data-dir` (`NANDI_RELAY_DATA_DIR`), so a restart keeps them. A revoked ID can't enroll again (403).
  - An open relay (`--open`) answers 404: there's no token to revoke.
- **Where:** nandi-relay `src/server.rs` (`revoke`, `reinstate`, `verify`, `operator_only`), `src/roster.rs`, `docs/relay-testing.md` ("A company relay").
- **Preconditions:** A relay with `NANDI_RELAY_SECRET`, two Nandis enrolled on it.
- **Steps:**
  1. Revoke one Nandi's instance ID with the secret.
  2. Watch that Nandi's Network › Connection, and the other Nandi's People.
  3. Restart the relay.
- **Expected:**
  - Step 1: 200 `{"revoked": true}`. The relay logs `- connection closed (… reason=revoked …)` within a second.
  - Step 2: the revoked Nandi can't reconnect ("Invalid or missing instance token"). The other Nandi no longer lists it.
  - Step 3: still refused after the restart.
- **Edge / negative cases:**
  - A wrong or missing bearer token gets 401.
  - Reinstating lets the instance register again with its old token.
  - Other instances are untouched.
- **Automation:** Covered by: nandi-relay `tests/relay.rs` (`a_revoked_instance_is_closed_at_once_and_stays_out`, `a_revoked_instance_cannot_enroll_again`) | Layer: api | Gap: none for the relay; no UI for the operator.
- **Needs relay:** yes
- **Status:** Shipped

### RL-31 — Relay over TLS, trusting a company's own CA

- **What:**
  - **Relay:** `--tls-cert` and `--tls-key` (or `NANDI_RELAY_TLS_CERT` and `NANDI_RELAY_TLS_KEY`) serve `wss://` and `https://`. `/health` reports `"tls": true`. A certificate and key that don't match stop the relay from starting.
  - **App:** trusts public CAs, the operating system's certificate store (where a company installs its own CA), and the PEM file in `NANDI_RELAY_CA_FILE`.
- **Where:** app: `services/relay/tls.rs` (client), `client.rs` (`connect_async_tls_with_config` with `tls::connector`); nandi-relay: `src/tls.rs`, `src/server.rs` (`serve_with`).
- **Preconditions:** A certificate for the relay's host, signed by a CA the test machines trust (or the test CA in `src-tauri/src/services/relay/testdata/` with `NANDI_RELAY_CA_FILE` pointing at `ca.pem`).
- **Steps:**
  1. Start the relay with the certificate and a secret.
  2. Connect a Nandi to `wss://<host>:8787/relay`.
  3. Connect another with neither the CA installed nor `NANDI_RELAY_CA_FILE`.
- **Expected:**
  - Step 2: Connected. **Test connection** shows the round trip.
  - Step 3: the connection fails with a certificate error, shown on the Connection view.
- **Edge / negative cases:** a plain `ws://` connection to the TLS port fails.
- **Automation:** Covered by: `relay/tests.rs` (`the_client_trusts_a_company_ca_for_wss`), nandi-relay `tests/relay.rs` (`the_relay_serves_wss_to_clients_that_trust_its_ca`, `a_bad_certificate_stops_the_relay_from_starting`) | Layer: api | Gap: the OS-store path isn't tested (it needs a CA installed on the machine).
- **Needs relay:** yes
- **Status:** Shipped

### RL-32 — Warning for an unencrypted relay on another machine

- **What:** Network › Connection says the relay isn't encrypted when its address is `ws://` and not this machine (`localhost`, `127.x.x.x`, `::1`). It shows while typing the address and under a saved one.
- **Where:** `src/components/views/ConnectionView.tsx` (`UnencryptedWarning`), `src/components/onboarding/identity-model.ts` (`isUnencryptedRemoteRelay`), `services/relay/tls.rs` (`is_unencrypted_remote`, the same rule), `network.connection.unencrypted` in `locales/*.json`.
- **Preconditions:** None.
- **Steps:** type `ws://192.168.1.10:8787/relay`, then `ws://127.0.0.1:8787/relay`, then `wss://relay.example/relay`.
- **Expected:** the warning shows only for the first.
- **Automation:** Covered by: `identity-model.test.ts` ("flags plain ws:// relays on other machines only"), `tls.rs` (`plain_ws_is_flagged_only_off_this_machine`) | Layer: unit | Gap: no render test of the warning.
- **Needs relay:** yes
- **Status:** Shipped

### RL-33 — Relay limits, container image and Helm chart

- **What:**
  - **Limits:** each connection may send 600 messages a minute (`NANDI_RELAY_MAX_MESSAGES_PER_MINUTE`). Past that, the relay answers `rate_limited` and drops the message.
  - **Image:** nandi-relay's `deploy/Dockerfile` builds `nandi-relay` into a small Debian image that runs as a non-root user, with `/data` for enrolments and revocations.
  - **Helm:** nandi-relay's `deploy/helm/nandi-relay/` runs one relay. The secret and join code come from an existing Secret, and a licence file from another (`licence.existingSecret`). TLS is either on an ingress or served by the relay (`tls.existingSecret`). It has a persistent volume for `/data`, and health probes.
- **Where:** nandi-relay: `src/server.rs` (`take_message`), `deploy/Dockerfile`, `deploy/helm/nandi-relay/`.
- **Preconditions:** Docker; a Kubernetes cluster for the chart.
- **Steps:**
  1. `docker build --build-context spec=../nandi-spec -f deploy/Dockerfile -t nandi-relay:dev .` from nandi-relay's root, then run it with a secret.
  2. `helm install relay deploy/helm/nandi-relay` (in nandi-relay) after creating the `nandi-relay-auth` Secret.
  3. Connect two Nandis through it.
- **Expected:** both connect, find each other and delegate. Restarting the pod keeps enrolments and revocations.
- **Automation:** Covered by: nandi-relay `tests/relay.rs` (`messages_past_the_limit_are_dropped`) | Layer: api | Gap: the chart is only linted and rendered (`helm lint`, `helm template`); it hasn't been installed on a cluster.
- **Needs relay:** yes
- **Status:** Shipped

### RL-35 — A relay's user limit (20 free, more with a licence)

- **What:** A relay serves 20 users free; a licence file from VaultDB.ai raises that to its seats until it expires. Users are the enrolled instance IDs that aren't revoked. The check lives only in the relay, never in the app.
  - A new ID past the limit is refused with the relay's reason: 402 from `/relay/enroll` and `/relay/token`, an `unauthorized` error on registration (tokens can't bypass it). On an open relay, distinct registered IDs count.
  - The app shows that reason on Network › Connection ("The relay refused us: This relay is at its free limit of 20 users…").
  - Current users are never cut off; after a licence expires, only new ones past 20 are refused. Revoking a user frees their seat.
  - `--licence FILE` or `NANDI_RELAY_LICENCE_FILE`; a file that doesn't verify stops the relay from starting. The startup log and `/health` (`users`, `seats`) show the count.
- **Where:** nandi-relay: `src/roster.rs`, `src/licence.rs`, `src/server.rs` (`enroll`, `issue_token`, `register`), `src/standalone.rs` (`seat_policy`); app: `services/relay/mod.rs` (`enroll`), `client.rs` (the refusal reason).
- **Preconditions:** A relay with a secret and a join code, 20 users enrolled (or a `relay.db` seeded with 20 users).
- **Steps:**
  1. Connect a 21st Nandi with a fresh home.
  2. Restart the relay with a licence file for more seats; connect it again.
- **Expected:**
  - Step 1: the Connection view shows the free-limit reason; the 20 others keep working.
  - Step 2: it enrolls and connects.
- **Edge / negative cases:** an expired licence keeps the current users and refuses new ones ("expired on …"); a forged or edited licence file stops the relay at startup.
- **Automation:** Covered by: nandi-relay `tests/seats.rs`, `src/roster.rs`, `src/licence.rs`, `src/standalone.rs` (`refuses_to_start_with_a_licence_that_does_not_verify`); app `relay/tests.rs` (`a_full_relay_s_reason_reaches_the_app`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-34 — Everyone on the company's relay is trusted

- **What:** When this Nandi's relay connection carries an instance token (the relay needs a secret, so only enrolled instances are on it) and the relay isn't a public one, every member the relay lists is trusted, as if marked **Trusted peer**, unless the owner turns off **Trust everyone on this relay** (Network › Connection, `trust.relayMembers`, on by default). Pause and Block still apply. Where it can't apply the switch shows off and disabled, whatever was saved, with the reason: no relay yet ("Applies once you join your company's relay…"), an open relay, or the public relay ("Anyone with an email can join the public relay, so you trust people one by one.", `RelayTrust.public`); the switch and the words always agree. Revoking someone on the relay (RL-30) takes them off it, so they lose this trust too.
- **Where:** `services/team/trust.rs` (`is_trusted`, `trusted_by_relay`, `on_a_secured_relay`, `relay_trust`, `set_trusts_relay_members`), used by `a2a/executor.rs` (agent and workflow trust checks) and `team/trust_requests.rs`. Commands `get_relay_trust`, `set_relay_trust`; HTTP `GET`/`POST /api/rpc/set_relay_trust`. UI: `ConnectionView.tsx` (`RelayTrustCard`).
- **Preconditions:** A relay with `NANDI_RELAY_SECRET` and a join code, three Nandis enrolled on it.
- **Steps:**
  1. From one Nandi, ask another's agent for a capability set to **Ask** (not Allow), without marking each other trusted.
  2. Block the sender on the receiving side, and repeat.
  3. Turn **Trust everyone on this relay** off on the receiver, and repeat.
- **Expected:** Step 1 goes through. Steps 2 and 3 are refused (step 3 becomes a trust request).
- **Automation:** Covered by: `trust.rs` (`everyone_on_a_secured_relay_is_trusted_unless_turned_off`), `relay/public_tests.rs` (`signing_in_links_this_nandi_keeps_the_token_and_connects`: not secured, `public`) | Layer: unit | Gap: the three-Nandi run hasn't been done.
- **Needs relay:** yes
- **Status:** Shipped

### RL-39 — Device keys, pinned by the relay

- **What:** Each Nandi makes its own keys (Ed25519 signing, X25519 sealing) the first time it uses a relay and keeps them in the OS keyring (`nandi.instance.keys`). It sends the public keys when it enrolls and registers. The relay pins them for the instance ID; other keys for the same ID are refused (`key_mismatch`) until the operator rekeys it.
  - The app shows "The relay has other keys for this device…" on Network › Connection and counts as not connected.
  - `POST /relay/rekey {"instanceId"}` with the secret clears the pin and writes an `audit` row; the next keys are pinned.
  - An open relay keeps the latest keys and refuses none.
- **Where:** app `services/relay/device_keys.rs`, `client.rs` (`device_keys`, `key_mismatch`), `join.rs` (`enroll`), `ConnectionView.tsx`; nandi-relay `src/server.rs` (`pin_keys`, `rekey`), `src/store/keys.rs`; nandi-spec `crates/nandi-seal`.
- **Preconditions:** A relay with a secret and a join code; one Nandi joined.
- **Steps:**
  1. Join the relay; check `relay.db` `members.signing_key` is set for this instance ID.
  2. Delete the keyring entry `nandi.instance.keys` (as after a reinstall) and restart Nandi.
  3. Run `POST /relay/rekey` for the instance ID, then reconnect.
- **Expected:**
  - Step 1: connected; the keys are pinned.
  - Step 2: Connection shows the key message; person-to-person actions say "not connected"; everything local works.
  - Step 3: connected again with the new keys pinned; the relay's `audit` table has a `rekey` row.
- **Edge / negative cases:** an unreadable keyring never makes new keys; malformed keys are refused (400 at enrollment).
- **Automation:** Covered by: app `relay/keys_tests.rs` (`registration_carries_the_keys_and_the_relay_pins_them`, `other_keys_read_as_key_mismatch_until_the_operator_rekeys`), `relay/device_keys.rs`; nandi-relay `tests/keys.rs`; nandi-spec `nandi-seal` tests | Layer: api | Gap: the reinstall run by hand hasn't been done.
- **Needs relay:** yes
- **Status:** Shipped

### RL-40 — "Their key changed" on a member's profile

- **What:** Nandi pins each member's keys the first time the relay's directory shows them, or the first time it seals for them or opens something from them (asked of the relay). If the relay later shows other keys for that member, sending to them stops (ask, assign, delegate, results, clarifications, reassignment, handover) until the user accepts the new key on their profile.
  - The profile shows the key's short fingerprint (`Key 3F2A 91C0 7B44 E1D9`), or a calm "Their key changed" notice with the old and new fingerprints and **Accept new key**.
  - A refused send reads "Their key changed. Open their profile…".
  - A result or message from their new device is refused too (it isn't signed by the pinned key, RL-42), and an open profile updates on its own (`network://member_key_changed`).
- **Where:** `services/network/peer_keys.rs` (`observe`, `pinned`, `learn`, `PinnedKeys`, `accept`), `network/mod.rs` (`refresh_with`), `relay/client.rs` (the key book), `team/roster.rs` (`MemberProfile.key`); command `accept_peer_key`, HTTP `POST /api/rpc/accept_peer_key`; UI `MemberKeyNotice.tsx` on `MemberProfileView`, `agent-sync.ts`.
- **Preconditions:** Two Nandis on a relay with a secret, each seeing the other in People.
- **Steps:**
  1. Open the other member's profile and note the key.
  2. On the other Nandi, reset its keys and have the operator rekey it (RL-39); refresh People.
  3. Ask that member something.
  4. Accept the new key on their profile and ask again.
- **Expected:**
  - Step 1: the fingerprint shows.
  - Step 2: the profile says "Their key changed" with both fingerprints.
  - Step 3: refused with "Their key changed…".
  - Step 4: the notice is gone and the request goes through.
- **Edge / negative cases:** the old key shown again drops the change; nothing to accept when the key didn't change.
- **Automation:** Covered by: `peer_keys.rs` tests, `relay/keys_tests.rs` (`a_member_whose_key_changed_is_blocked_until_accepted`), `relay/sealed_tests.rs` (`a_result_from_a_replaced_device_is_refused_and_its_key_flagged`), `MemberProfileView.test.tsx` (Key), `friendly-error.test.ts` | Layer: unit + api + ui | Gap: the two-Nandi run hasn't been done.
- **Needs relay:** yes
- **Status:** Shipped

### RL-41 — The relay's task ledger (operator view)

- **What:** The relay records the metadata of the tasks it routes (who asked whom, capability, state, typed reason, ETA, progress, place in line, deadline, times, duration), never their content, and keeps each task 30 days after it ended (`--task-retention-days`). The operator reads it with the secret: `GET /team/tasks?member=&state=&capability=` and `GET /team/tasks/{id}`. Nothing changes for the app.
- **Where:** nandi-relay `src/ledger.rs`, `src/store/tasks.rs`, `src/server.rs` (`team_tasks`, `team_task`).
- **Preconditions:** A relay with a secret; two Nandis on it.
- **Steps:**
  1. Delegate a task from one Nandi to the other and let it finish.
  2. `curl -H "Authorization: Bearer $NANDI_RELAY_SECRET" http://<relay>/team/tasks/<task id>`.
  3. Search `relay.db` (and its `-wal`) for a word from the request.
- **Expected:** Step 2 shows the task `completed` with its events and duration. Step 3 finds nothing.
- **Edge / negative cases:** without the secret the endpoints answer 401; an open relay has none (404).
- **Automation:** Covered by: nandi-relay `tests/ledger.rs` (states and duration, no content columns, retention sweep, operator auth), `tests/sealed.rs` (a plaintext marker never in the database, WAL or backups), `src/ledger.rs` | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-42 — Content sealed end to end

- **What:** Everything one person's Nandi sends another is sealed for that person's device (`nandi-seal-v1`, relay protocol 4): a task's input (gateway requests too), its result and error, clarification questions and answers, notes, reasons, late results and a failed duty's error. The relay forwards the envelopes and sees only metadata; it can't open them.
  - Something that doesn't open, isn't for this task or this person, or isn't signed by the sender's pinned key is refused: a toast says "Something they sent couldn't be verified…", nothing in it is used, and a request you sent with it fails with that reason.
  - A member who has no keys at the relay can't be sent content: "They haven't set up their keys yet…". Content over 1 MiB isn't sent: "This is too big to send… Share files as links".
  - Reassigning keeps the requester's signature: the provider reseals the request for the new provider and the relay delivers it; without the opened request, the requester (the author) seals it again.
  - What waits at the relay for someone offline (requests, results, notes, questions) waits sealed.
  - Your own agents (local delegation) are unchanged and never sealed.
- **Where:** app `services/relay/sealing.rs` (`Sealer`), `opened.rs`, `client.rs` (keys, the inbound queue, `reassign`, `reseal_as_author`), `a2a/reassign.rs`, `src/lib/friendly-error.ts`, `agent-sync.ts` (`relay://seal_refused`); nandi-spec `nandi-protocol` (`content`, `Envelope`, `SealedTask`, `ResultReport`) and `nandi-seal`; nandi-relay `src/server.rs` (the envelope cap, `PublicKeysQuery`).
- **Preconditions:** Two (for reassignment, three) Nandis on a relay with a secret and a data directory.
- **Steps:**
  1. Ask the other person something with a word you'll search for, have them ask a clarifying question back and answer it, then let it finish.
  2. Search the relay's `relay.db`, `relay.db-wal` and a backup (`POST /relay/backup`) for the word.
  3. On the provider, reassign a task to the third Nandi and let it finish there.
  4. Reset the provider's keys and rekey it at the relay (RL-39); have it send a result for an open task.
- **Expected:**
  - Step 1: the request, question, answer and result read normally on both sides.
  - Step 2: the word isn't found; the task's metadata is (RL-41).
  - Step 3: the third Nandi runs the original request; the result reaches the requester.
  - Step 4: the requester refuses it with the "couldn't be verified" toast, and the provider's profile says "Their key changed" (RL-40).
- **Edge / negative cases:** a duplicate delivery is dropped; an envelope moved to another task, capability or person is refused; with the relay down nothing is sealed or sent (RL-36).
- **Automation:** Covered by: app `relay/sealed_tests.rs` (every frame through a WebSocket tap in front of a real relay, its database and WAL: no content in the clear; reassignment with the requester's signature; work and a result waiting at the relay; missing keys; a replaced device), `relay/sealing_tests.rs`, `a2a/tests.rs` (the mock hub seals and opens every message), `friendly-error.test.ts`; nandi-relay `tests/sealed.rs`; nandi-spec `nandi-protocol` and `nandi-seal` tests | Layer: unit + api | Gap: none (live three-Nandi run 2026-11-05, see `docs/testing/team-brain-live-run-2026-11-05.md`).
- **Needs relay:** yes
- **Status:** Shipped

### RL-43 — Store-and-forward: work and answers wait at the relay

- **What:** The relay keeps everything for a Nandi in its mailbox until that Nandi says it has it (relay protocol 6): a task for someone offline, a result for a requester who went away, notes, questions and answers, what the relay did with a task (failed over, reassigned, ended). A Nandi catches up when it connects (`InboxSync`); nothing is polled. With the relay down, a question or an assignment to someone waits in this Nandi's outbox and goes when the relay is back; any other request to someone else says "Connect to your company's relay".
- **Where:** app `services/relay/client.rs` (`Deliver`, `Ack`, `InboxSync`, acks only once a handler has it), `a2a/queue.rs` (the outbox, `flush` on connect and every sweep), `a2a/task.rs` (`on_event`); nandi-relay `src/mailbox.rs`, `src/dispatch.rs`, `src/store/mail.rs`.
- **Preconditions:** Two Nandis on a relay with a data directory.
- **Steps:**
  1. Quit B. On A, ask B something. 2. Start B. 3. Answer on B, quit A before the answer arrives, then start A.
  2. Stop the relay. On A, assign B some work. Start the relay.
  3. Restart the relay while a task is open.
- **Expected:**
  - Step 1: A's thread says B is offline and the relay keeps the request. Step 2: B gets it.
  - Step 3: A gets the answer when it starts.
  - Step 4: the assignment waits on A ("waiting for the relay"), and B gets it once the relay is back.
  - Step 5: nothing is lost; each delivery arrives once.
- **Edge / negative cases:** a delivery that comes twice (live, then from the sync) is handled once; a delivery nothing here handles yet isn't acked and comes again; live-only messages (a status question, "still working") aren't stored.
- **Automation:** Covered by: app `relay/sealed_tests.rs` (`work_for_someone_offline_waits_at_the_relay_and_so_does_the_result`), `a2a/tests.rs` (`the_outbox_flushes_in_priority_order_once_the_relay_is_back`, `a_delegation_to_an_offline_provider_waits_at_the_relay`, `an_assignment_stays_open_until_its_person_ends_it`); nandi-relay `tests/dispatch.rs` (`work_for_someone_offline_waits_and_the_result_waits_for_the_requester`, `unacked_mail_comes_again_and_survives_a_restart`, `a_cancel_takes_back_an_undelivered_task`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-44 — The relay's management API (operator secret)

- **What:** The relay's operator manages the team over HTTP with the relay secret (`Authorization: Bearer $NANDI_RELAY_SECRET`): `GET /team/members` (seat, online, last seen, availability and whether the operator set it, keys, open tasks), `PUT`/`DELETE /team/members/{id}/availability` (someone out sick: away, with who covers; routing and people search follow it), `GET /team/tasks?open=true`, `GET /team/tasks/{id}` (events and attempts), `POST /team/tasks/{id}/reassign {"target","by"}` (a manager's move: it waits for the provider's Nandi to reseal, 202), `GET /team/stats?days=` (per member and capability; members never see these), `GET /team/watches`, `GET /team/audit`. Every change is audited. Nothing changes in the app.
- **Where:** nandi-relay `src/admin.rs`, `src/dispatch.rs` (`manager_reassign`), `src/store/keys.rs` (audit).
- **Preconditions:** A relay with a secret, three Nandis.
- **Steps:** 1. List members. 2. Set B away with C covering, then ask B for something from A. 3. Move an open task of B's to C. 4. Read the stats and the audit.
- **Expected:** Step 2: the request goes to C with "B is away; C covers". Step 3: B's Nandi reseals it, C gets it, A's thread says "Reassigned to C by {by}". Step 4: both changes are in the audit.
- **Edge / negative cases:** without the secret every endpoint answers 401; an open relay has none (404); a move to the requester or someone who had it is refused (409).
- **Automation:** Covered by: nandi-relay `tests/admin.rs`, `tests/dispatch.rs` (`a_managers_move_waits_for_the_reseal`), `src/admin.rs` | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-45 — Join the public relay with the connected email

- **What:** People without a company relay join VaultDB.ai's public relay with the Outlook or Gmail they connected (the locked identity, ST-34), not with a join code or a sign-in. Onboarding's network step (ON-09) and Network › Connection (while no relay is set: a **Public relay** card with **Join the public relay**) share one flow. It asks the relay's `/health` (`public`, `identity`: whose ID tokens it accepts) and shows the free limits ("up to 10 groups, 50 people per group and 500 messages a day"), "You join as <email>, the email this Nandi is connected with." and one **Join the public relay** button. Joining posts this Nandi's ID, device public keys, a fresh ID token of the identity's mailbox and the machine hash (the relay's salt) to `/relay/public/join`; the relay checks the token and locks the email to the device key. The token is kept exactly like an enrolled one (`relay_state`, never `config.json` or the UI), the account is remembered (`relay_state.relay_account`) and Nandi connects: "Joined as <email>. Connecting to the public relay…". With no email connected the card says "Connect your Outlook or Gmail in Settings › Email to join groups on the public relay." with **Open Settings › Email**, and there is no other way in; a disconnected or ended mailbox says to connect it again. Every refusal code has its own calm line (`PublicRelayRefusal`, `publicRelay.refusal.*`) with the relay's words under it: `id_token_invalid`, `id_token_expired`, `email_unverified`, `identity_unavailable`, `account_suspended`, `already_linked`, `key_mismatch`, `device_limit`, `machine_limit`, `signup_limit`, `revoked`, `invalid_request`, `unauthorized`, `no_account`, `not_public`, anything else. An unreachable relay is never an error: "The public relay isn't reachable right now — you can join later from Network › Connection." with **Try again**. The URL is `wss://relay.vaultdb.ai/relay`, or `NANDI_PUBLIC_RELAY_URL` (staging, tests).
- **Where:** `services/relay/public.rs` (`offer`, `join`, `PUBLIC_RELAY_URL`, `PUBLIC_RELAY_URL_ENV`), `services/identity/` (`fresh_id_token`, `machine_id`), `services/relay/mod.rs` (`BuiltInRelay::public_url`, `RelayStatus.publicRelayUrl`), commands `get_public_relay_offer`, `join_public_relay`; HTTP `GET /api/relay/public`, `POST /api/relay/public/join`; UI `components/network/PublicRelayJoin.tsx`, `public-relay-model.ts`, `ConnectionView.tsx` (`PublicRelayCard`), `Step3_Network.tsx`. The relay: nandi-relay `--public` (README "A public relay").
- **Preconditions:** A relay run with `--public` and `NANDI_RELAY_GOOGLE_CLIENT_ID` / `NANDI_RELAY_MICROSOFT_CLIENT_ID` set to the app build's mail client IDs; the app started with `NANDI_PUBLIC_RELAY_URL` pointing at it.
- **Steps:**
  1. Fresh home, no email connected: in onboarding choose **Join the public relay**.
  2. Connect Gmail (or Outlook) in the Email step, then **Join the public relay**.
  3. Restart the app.
  4. Second fresh home on another computer, same email connected: join.
  5. A third home with an email of another account under the other provider but the same address: join.
  6. Stop the relay and open Network › Connection › **Join the public relay** on a fresh home with an email.
- **Expected:**
  - (1) The "Connect your Outlook or Gmail…" line and **Open Settings › Email**; no join button.
  - (2) "You join as <email>…", then "Joined as <email>"; Connection shows the relay connected, its address without the token.
  - (3) It reconnects with the kept token; nothing is asked again.
  - (4) The same account: its Account card (RL-46) lists both Nandis.
  - (5) The same account again (the same verified email).
  - (6) The calm unreachable line and **Try again**; nothing else in Nandi changes.
- **Edge / negative cases:** joining the public relay's address as a company relay answers `publicRelay` ("… is a public relay: join it with your connected email …"); a mailbox disconnected after the identity was locked ("Connect <email> again …"); a token the relay doesn't accept (`id_token_invalid`); a Nandi already joined with another email (`already_linked`); the account at its Nandis (`device_limit`); a 6th Nandi or a 4th account from one computer (`machine_limit`); a reinstalled device (`key_mismatch`).
- **Automation:** Covered by: Rust `relay/public_tests.rs` (the real `nandi-relay` in public mode trusting its fake identity provider: `joining_with_the_connected_email_keeps_the_token_and_connects` (needsEmail first, offer, joined, token kept, connected, groups visibility, `publicRelay`), `refusals_come_back_typed_and_change_nothing`, `a_disconnected_mailbox_needs_connecting_again`, `every_refusal_code_has_its_kind`, `an_unreachable_public_relay_answers_calmly_and_quickly`), nandi-relay `tests/public.rs` (tokens, caps per computer, same email under both providers), `offline_suite.rs` (`an_unreachable_public_relay_never_blocks_onboarding`); vitest `public-relay-model.test.ts`, `PublicRelayJoin.test.tsx`, `ConnectionView.test.tsx`, `OnboardingView.test.tsx` | Layer: unit + api | Gap: real Google and Microsoft ID tokens against the live `relay.vaultdb.ai` are manual.
- **Needs relay:** yes
- **Status:** Partial (the public relay at `relay.vaultdb.ai` isn't live yet; everything runs against a self-run `--public` relay)

### RL-46 — The public relay's Account card

- **What:** On the public relay, Network › Connection shows **Public relay account**: "Joined as <email>", this account's Nandis (instance IDs, "(this Nandi)" marked), the free limits and today's usage ("Groups 2 of 10", "People per group Up to 50", "Messages today 37 of 500") and when today's count resets, in the user's own clock. A note says "On a public relay you see only people you share a group with." with **Open Groups**. While the relay doesn't answer, the card names the account as kept and says "Your account's details show when the public relay answers." Refreshed every minute. On any other relay there is no card.
- **Where:** `services/relay/public.rs` (`account`: `GET /relay/account?instanceId=` with the token as bearer; `PublicRelayAccount`), command `get_public_relay_account`, HTTP `GET /api/relay/public/account`, UI `components/network/PublicRelayAccountCard.tsx`, `hooks/useRelay.ts` (`usePublicRelayAccount`).
- **Preconditions:** RL-45 joined.
- **Steps:** 1. Open Network › Connection. 2. Create a group and send a request. 3. Stop the relay and reopen.
- **Expected:** 1. The card as above. 2. Groups and messages today go up by what the relay counts. 3. The kept account and the calm line.
- **Automation:** Covered by: `relay/public_tests.rs` (`joining_with_the_connected_email_keeps_the_token_and_connects`, `past_the_daily_limit_a_group_change_says_when_it_resets`: account, Nandis, limits, usage), `ConnectionView.test.tsx` (the card, the unavailable case) | Layer: unit + api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RL-47 — Limits per person and the daily message limit

- **What:** A relay's limits count per person (on the public relay, an account: all their Nandis share them). Past a group limit the relay's `group_limit` reads "That's past this relay's group limits, which count per person (all your Nandis together): …". Past the daily message limit (500 on the public relay) the relay answers `daily_limit` with its reset (00:00 UTC); the client retells it as `daily_limit@<reset ms>: <the relay's words>` for a request (its `refused` event), a reassignment (`ReassignResult`) and a group change (`GroupReply` refused). Where the action was taken it shows calmly, not as an error: "You've sent today's messages on the public relay. You can send again from <time>." (the ask composer's toast, a request's outputs, a group action's toast).
- **Where:** `services/relay/limits.rs` (`DAILY_LIMIT`, `daily_limit`), `relay/client.rs` (`Shared::retell`), `groups/mod.rs` (`refusal`), `DAILY_LIMIT` in `bindings.ts`, `lib/friendly-error.ts` (`dailyLimit`, `dailyLimitResetsAt`, `friendlyErrorParams`), `toast-error.ts`, `network/group-toast.ts`, `AskMemberComposer.tsx`, `RequestDetailView.tsx`.
- **Preconditions:** A `--public` relay with `NANDI_RELAY_MESSAGES_PER_DAY=1`, signed in (RL-45).
- **Steps:** 1. Create a group. 2. Create another. 3. Ask a group-mate something.
- **Expected:** 1. Created. 2 and 3. The calm daily-limit line with the local reset time; nothing else breaks, and leaving a group still works.
- **Edge / negative cases:** a clarification past the limit gets only the relay's `Error` frame (logged; no place in the UI shows it yet).
- **Automation:** Covered by: `relay/public_tests.rs` (`past_the_daily_limit_a_group_change_says_when_it_resets`), `relay/limits.rs` tests, `relay/client.rs` (`coded_refusals_are_retold_once`), vitest `friendly-error.test.ts`, `group-toast.test.ts` | Layer: unit + api | Gap: a request refused past the limit against a real relay (two signed-in people sharing a group).
- **Needs relay:** yes
- **Status:** Shipped

### RL-24 — WebSocket event stream (`/ws`)

- **What:** Every bus event goes out as a text frame `{"event":"<name>","payload":{…}}`. Authentication is by cookie or `?token=`. A client that falls behind gets `{"event":"nandi://resync","payload":{"missed":N}}` and should reload its snapshots; the web client also emits resync after a reconnect.
- **Where:** `src-tauri/src/http/ws.rs`, `src/lib/client/web.ts`.
- **Preconditions:** The server is running.
- **Steps:** 1. Connect with `wscat -c "ws://127.0.0.1:<port>/ws?token=<t>"`. 2. Trigger a relay status change or start an agent.
- **Expected:** You receive frames for `relay://status`, `agent://…` and the other events.
- **Edge / negative cases:** Connecting without a token gets 401 on the upgrade.
- **Automation:** Covered by: `http/mod.rs` (`websocket_streams_events_after_auth`), `src/lib/client/events.test.ts` | Layer: api | Gap: no lag/resync test.
- **Needs relay:** no
- **Status:** Shipped

### RL-25 — Frontend served over HTTP

- **What:** The server serves `dist/`: embedded in release builds, read from disk in debug builds. Unknown paths fall back to `index.html`. `index.html` is sent with `no-cache` and hashed assets with an immutable one-year cache. When the bundle hasn't been built, the response is 503 "The frontend isn't built. Run `pnpm run build`…". In the browser, desktop-only features (file picker attachments, windows, companion) report as unsupported.
- **Where:** `src-tauri/src/http/static_files.rs`, `src/lib/client` (`isDesktop()`).
- **Preconditions:** `pnpm run build` has been run (for debug builds).
- **Steps:** 1. Open the access link in a browser. 2. Navigate to a deep link and reload. 3. Try to attach a file in chat.
- **Expected:** The app loads and the deep link reloads correctly. Attachments are reported as unsupported.
- **Edge / negative cases:** A debug build with no `dist/` returns 503.
- **Automation:** Covered by: `http/mod.rs` router tests | Layer: ui-e2e (browser) | Gap: no browser end-to-end test.
- **Needs relay:** no
- **Status:** Shipped

### RL-26 — HTTP API as an automation surface

- **What:** Nearly every feature is reachable as REST under `/api/*` (body and response JSON in camelCase, errors `400 {"error"}`, frozen-but-unimplemented routes `501`), so tests and scripts can drive Nandi without the UI. Route groups, from `src-tauri/src/http/routes.rs` plus the modules merged in `http/mod.rs`:
  - Core: `/agents…`, `/approvals/pending`, `/queue/{runId}/…`, `/snapshots/…`, `/runs…`, `/audit/{kind}`, `/workflows…`, `/workflow-templates`, `/workflow-runs/{id}/approve`, `/studio/conversations…`, `/conversations/{id}`, `/attachments/{id}`, `/preferences`, `/llm-keys…`, `/ollama`, `/default-model`, `/free-trial`, `/egress/…`, `/repo/status|rebuild`, `/emergency-data/…`.
  - This area: `/identity/*`, `/teams…`, `/teams/{id}/roster`, `/members/…`, `/trust/requests…`, `/trust/policies`, `/shared/with-me|by-me|capabilities|pins`, `/network/snapshot|refresh|members/{instance}[/trust]`, `/relay/status|url|agents|test|instances`, `/delegation/providers|tasks|tasks/{id}|history|pending-clarifications`, `/delegation/{id}/clarify|escalate/{q}`, `/agents/{id}/delegation`, `/agents/{id}/clarification-policy`, `/tasks…` (list, detail, cancel, status, events, messages, reassign, reassign-candidates), `/eta/{capability}`, `/remote/*`, `/config/llm[/reload]`.
  - Other modules (owned by other sections): marketplace, mascot, triggers (including unauthenticated `/api/triggers/webhook|device/…`), repo, pilots, connectors, images, capability, capabilities, tools, capability registries, analysis, workflow builder, marketplace search, companion, palette, memory, requests, policies, gateway, harnesses, harness sessions, aws-auth, bedrock, ask, studio, work, manual steps.
  - Events: `GET /ws` (RL-24).
- **Where:** `src-tauri/src/http/*.rs`.
- **Preconditions:** The server is running and you have the token.
- **Steps:** 1. Two headless instances plus a relay. 2. `PUT /api/relay/url`. 3. `PUT /api/gateway/config` (relay enabled and discoverable). 4. `POST /api/rpc/set_trust_relationship {"addedAsTrustedPeer":true}` on B. 5. `POST /api/delegation/tasks`. 6. Poll `POST /api/rpc/get_task_events`.
- **Expected:** The cross-instance delegation completes using only the API.
- **Edge / negative cases:**
  - Service errors return 400 with the same message the IPC command returns.
  - A bad enum in a query returns 400 naming the value.
  - Unmounted paths return the SPA, not 404.
- **Automation:** Covered by: `http/mod.rs` (`routes_map_onto_services`, `wave_2a_routes_are_mounted`, `wave_2b_redesign_routes_are_mounted`, `wave_final_routes_are_mounted`, `wave_a_routes_are_mounted`, `wave_b_routes_are_mounted`, `wave_cd_routes_are_mounted`, `service_errors_are_400_with_the_message`), `http/routes.rs` tests | Layer: api | Gap: there is no OpenAPI spec or generated client for scripts. `docs/developer/transports.md` lists only the core REST routes.
- **Needs relay:** no
- **Status:** Shipped

### RL-36 — "Connect to your company's relay" (the typed not connected)

- **What:** A person-to-person action with no relay connected or configured answers one typed error, `RelayUnavailable`, instead of ad-hoc strings. The UI shows one calm state for it, "Connect to your company's relay" with **Open connection settings**, only in person-to-person places: People (NW-03), Ask {name} (NW-08), Sharing (a line at the top while disconnected), Call (Library › Shared), delegating a capability, reassigning, and asking a provider for status. Local screens never show it, and it never shows as a raw error toast.
- **Where:** Rust `services/relay/unavailable.rs` (`RelayUnavailable`, `RELAY_UNAVAILABLE = "relay_unavailable"`, `connected`, `configured`), used by `relay/client.rs`, `relay/presence.rs`, `a2a/{task,reassign,clarification,delegator}.rs`, `a2a/mod.rs` (`delegate_task` to another Nandi's card), `relay/mod.rs` (`get_relay_instances`). The constant is exported to `src/lib/bindings.ts`. HTTP answers `503 {"error":"relay_unavailable: …"}` (`http/routes.rs` `ApiResult`). Frontend: `isRelayUnavailable` and the `relayUnavailable` error kind (`src/lib/friendly-error.ts`), `RelayConnectNotice` (`src/components/network/RelayConnectNotice.tsx`), `toastPersonError` (`src/lib/toast-error.ts`), `useRelayConnected` (`useRelay.ts`).
- **Preconditions:** No relay configured (or Disconnect), or a relay URL nothing listens on.
- **Steps:** 1. Open Network › People. 2. Open a member you saw before and try Ask. 3. Library › Shared › Call someone's agent. 4. Use Home, Agents, Automations, Library, Settings.
- **Expected:**
  - Steps 1 to 3 show "Connect to your company's relay"; the button opens Network › Connection.
  - Step 4: no relay notice or toast anywhere.
  - `POST /api/delegation/tasks` with another Nandi's target answers 503 with the `relay_unavailable` error; delegating to your own agents still works.
- **Edge / negative cases:**
  - A configured relay that is reconnecting keeps a question or an assignment to someone in the outbox (RL-43) instead of refusing it.
  - The team inbox (holds, assignments) decides locally; answers go out when the relay is back.
  - A call already waiting when the connection drops, when the relay forgets our registration (`not_registered`), or that never sees us registered answers `RelayUnavailable` too. A relay that is connected but slow still answers "The relay didn't … within 5s".
- **Automation:** Covered by: `relay/unavailable.rs` tests, `relay/client.rs` (`a_call_the_connection_lets_down_is_relay_unavailable`, `a_connection_that_never_registers_is_relay_unavailable`), `offline_suite.rs` (`with_no_relay_person_to_person_answers_not_connected`), `friendly-error.test.ts`, `NetworkView.test.tsx`, `MemberProfileView.test.tsx` | Layer: unit + api | Gap: no UI test for Sharing's line or the Call dialog.
- **Needs relay:** no
- **Status:** Shipped

### GR-01 — Network › Groups lists the relay's groups

- **What:** Network › Groups shows every group on the relay with its name, description and member count, my groups first with a **Member** or **Admin** badge. Nobody is in a group by default. Without a relay the tab shows "Connect to your company's relay", never an error.
- **Where:** `src/components/views/GroupsView.tsx`, `NavPages.tsx` (`groups` tab), `useGroups.ts`; command `list_groups`, `GET /api/groups`; Rust `services/groups/mod.rs` (`Groups::overview`).
- **Preconditions:** A relay with groups support (protocol 7) and two instances (RL-11).
- **Steps:** 1. Disconnect and open Network › Groups. 2. Connect and open it again. 3. On the second instance, create a group.
- **Expected:** Step 1: the calm connect state. Step 2: "No groups yet" or the list. Step 3: the first instance's list shows the new group (after `network://group_changed` or a refresh) without a badge, with "Ask a member for an invite".
- **Edge / negative cases:** A relay on protocol 6 refuses registration (version mismatch, RL-27), so the tab shows the connect state.
- **Automation:** Covered by: `groups::tests::without_a_relay_the_lists_are_empty_and_calls_say_so`, `groups-model.test.ts` | Layer: unit | Gap: no UI test for the list.
- **Needs relay:** yes
- **Status:** Shipped

### GR-02 — Create a group with a signed policy

- **What:** **Create group** asks for a name, a description and the policy, prefilled with a template whose comments explain each `requests:` field (`approval: always` by default). The creating Nandi picks the group ID (`g_` and 16 hex characters) and signs version 1 of the policy over it with this device's key. The creator is the first admin; their access level for the group starts at Ask first.
- **Where:** `GroupsView.tsx` (`CreateDialog`); `create_group`, `POST /api/groups`; `Groups::create`, `domain::group::POLICY_TEMPLATE`, `GroupPolicyText::parse`.
- **Preconditions:** Connected to a groups-capable relay.
- **Steps:** 1. Create "Finance" with the template. 2. Open `~/.nandi/repo/.nandi/groups/`. 3. Try a policy with `approval: sometimes`.
- **Expected:** Step 1: the group opens, "Verified, signed by {you}, version 1". Step 2: `<group-id>.md` (Nandi's frontmatter, then the policy text unchanged) and `access.yaml` with `<group-id>: ask-first`, both staged in git. Step 3: refused before anything reaches the relay, naming `requests`.
- **Edge / negative cases:** An ID the relay already has (`group_invalid`) is retried once with a new one. An empty name is refused.
- **Automation:** Covered by: `groups::tests::creating_signs_version_one_over_the_id_it_picks`, `relay::tests::on_the_real_relay_a_member_verifies_and_enforces_the_policy`, `domain::group::tests::*` | Layer: unit + api | Gap: no UI test of the dialog.
- **Needs relay:** yes
- **Status:** Shipped

### GR-03 — Join with a code, invites for me

- **What:** **Join with code** joins a group with an invite code. Invites addressed to me show at the top of Groups with **Accept** (joins with its code) and **Ignore** (hides it for this session). Joining keeps a copy of the group's policy and sets my access level to Ask first.
- **Where:** `GroupsView.tsx` (`JoinDialog`, invites section); `join_group`, `POST /api/groups/join`; `Groups::join`.
- **Preconditions:** A member made an invite (GR-05).
- **Steps:** 1. Paste a code and Join. 2. Have a member invite you by name, then Accept. 3. Join with a used-up code.
- **Expected:** 1–2: the group opens; its copy is in `.nandi/groups/`, and the author's key is pinned. 3: "That invite code isn't valid…".
- **Edge / negative cases:** Joining a group you're already in uses nothing and opens it.
- **Automation:** Covered by: `groups::tests::joining_keeps_a_verified_copy_and_leaving_drops_it`, `relay::tests::on_the_real_relay_a_member_verifies_and_enforces_the_policy`, `GroupsView.test.tsx` | Layer: unit | Gap: Ignore is not remembered across restarts.
- **Needs relay:** yes
- **Status:** Shipped

### GR-04 — Group detail: members, roles, Leave

- **What:** A group I'm in opens its detail: members with roles (admins first); admins can **Make admin** / **Make member** and **Remove** (asks first). The last admin can't be demoted. **Leave** (asks first) removes the group's copy and my access level.
- **Where:** `GroupDetail.tsx` (`MembersCard`, Leave); `set_group_role`, `remove_group_member`, `leave_group` (`/api/groups/{id}/members/{instance}`, `/api/groups/{id}/leave`).
- **Preconditions:** A group with two members.
- **Steps:** 1. As admin, make the other member an admin, then a member again. 2. Remove them. 3. Leave as the last admin while another member remains.
- **Expected:** 1–2 update the list for everyone (live, `network://group_changed`). 3: "A group needs an admin: make someone else an admin first".
- **Edge / negative cases:** A removed member's Nandi drops its copy on the next `GroupChanged` or reconnect.
- **Automation:** Covered by: `groups::tests::refreshing_drops_copies_of_groups_im_not_in`, `groups-model.test.ts` | Layer: unit | Gap: role changes untested against a real relay.
- **Needs relay:** yes
- **Status:** Shipped

### GR-05 — Invites: a person or a code

- **What:** Members invite someone they can see (one use) or make a code with a number of uses and days before it expires; the code is copied. Open invites are listed with Copy and **Revoke**.
- **Where:** `GroupDetail.tsx` (`InvitesCard`); `invite_to_group`, `revoke_group_invite`.
- **Preconditions:** A group you're in.
- **Steps:** 1. Create a code for 3 uses, 7 days. 2. Revoke it. 3. Invite a person.
- **Expected:** 1: "Invite code copied", listed with "Uses left: 3" and its expiry. 2: gone. 3: the person sees it under Invites for you.
- **Edge / negative cases:** The relay limits open invites (50 per group) and uses (500).
- **Automation:** Covered by: `relay::tests::on_the_real_relay_a_member_verifies_and_enforces_the_policy` (invite a person) | Layer: api | Gap: codes, revoke and expiry untested.
- **Needs relay:** yes
- **Status:** Shipped

### GR-06 — The policy: verified or fail closed; admins edit it

- **What:** The group's policy is shown read-only with "Verified, signed by {author}, version {N}", or, when its signature doesn't check out against the author's pinned key, "This group's policy couldn't be verified" (and work from that group is refused, GR-08). Admins **Edit policy** and **Sign and save**: the next version, signed by this device.
- **Where:** `GroupDetail.tsx` (`PolicyCard`); `set_group_policy`, `PUT /api/groups/{id}/policy`; `Groups::set_policy`, `domain::group::verify`, `groups::enforce::Enforcer::verified`.
- **Preconditions:** A group you administer.
- **Steps:** 1. Edit the policy to `approval: per-member` and save. 2. Edit `.nandi/groups/<id>.md` by hand (change the text).
- **Expected:** 1: version 2, verified, on every member's Nandi. 2: requests from that group's members are refused with "this group's policy couldn't be verified" until the next refresh rewrites the copy.
- **Edge / negative cases:** Another admin saving at the same time (`group_policy_stale`): signed again over the next version once. A member whose key changed (RL-40) can't verify their policies until accepted.
- **Automation:** Covered by: `groups::tests::a_policy_change_is_the_next_version_signed_by_me`, `groups::tests::a_forged_policy_shows_unverified`, `groups::enforce::tests::an_unverifiable_policy_fails_closed`, `domain::group::tests::a_signature_checks_out_only_for_its_group_version_and_text` | Layer: unit | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### GR-07 — My access level per group

- **What:** In a group's detail, **My access level** offers Fully automated, Ask first, Questions only, Send me info and None, each with a line on what it means. The stricter of my level and the group's policy applies; when the policy holds back what I picked, a note says so. Works offline (it's my own file, `.nandi/groups/access.yaml`).
- **Where:** `GroupDetail.tsx` (`AccessCard`); `get_group_access`, `set_group_access`; `domain::group::{AccessLevel, GroupAccess}`.
- **Preconditions:** A group you're in.
- **Steps:** 1. Pick Fully automated in a group whose policy is `approval: always`. 2. Pick Questions only.
- **Expected:** 1: saved, with "The group's policy is stricter: requests still wait for your approval". 2: `access.yaml` says `questions-only`.
- **Edge / negative cases:** A group missing from `access.yaml` counts as Ask first.
- **Automation:** Covered by: `domain::group::tests::access_levels_default_to_ask_first_and_round_trip`, `repos::group_repo::tests::*`, `groups-model.test.ts` | Layer: unit | Gap: no UI test.
- **Needs relay:** no
- **Status:** Shipped

### GR-08 — Group policies on incoming work

- **What:** A request from someone I share groups with follows, per group, the stricter of the group's `requests:` rules and my access level, and across groups the strictest. Refused requests tell the requester why ("Not accepted here. Finance: the owner takes only questions through this group"); held ones wait in my queue with the group's reason in the thread; Fully automated (when the policy allows) runs a clean request without asking. No shared group: unchanged.
- **Where:** `services/groups/enforce.rs`, `a2a/executor.rs` (`intake`), `intake/mod.rs` (`GroupSay`), `gateway/router.rs` (`handle_gateway_task`).
- **Preconditions:** Two instances in one group.
- **Steps:** 1. Policy `approval: always`; ask the other person's agent something. 2. Policy `approval: per-member`, their level Fully automated; ask again. 3. Their level Questions only; assign them work. 4. `capabilities: { deny: [review-*] }`; ask for `review-pr`. 5. `daily_limit: 1`; ask twice.
- **Expected:** 1: waits for their approval even if you're trusted. 2: runs at once. 3: refused with the reason. 4: refused. 5: the second is refused for the limit.
- **Edge / negative cases:** `hours:` outside the window holds instead of running. `always_ask` words (`send`) hold a matching capability. A suspicious request always waits.
- **Automation:** Covered by: `groups::enforce::tests::*`, `intake::tests::a_group_that_asks_holds_even_a_trusted_teammate`, `intake::tests::a_group_that_lets_it_run_approves_a_clean_request_from_anyone`, `domain::group::tests::*` | Layer: unit | Gap: the policy's `tools:`/`connectors:` blocks are not applied to the run itself yet; "Send me info" holds questions only (no separate information intake).
- **Needs relay:** yes
- **Status:** Partial (intake only, see Gap)

### GR-09 — A relay that shows only people you share a group with

- **What:** A relay started with `--visibility groups` tells Nandi so at registration. People then shows "This relay shows you the people you share a group with", Groups explains it, and a request to someone you share no group with is refused with "You don't share a group with them on this relay".
- **Where:** `relay/client.rs` (`Registered.visibility` into `RelayStatus.visibility`, `no_shared_group`), `NetworkView.tsx`, `friendly-error.ts` (`noSharedGroup`), `NO_SHARED_GROUP` in `bindings.ts`.
- **Preconditions:** A relay with `NANDI_RELAY_VISIBILITY=groups`.
- **Steps:** 1. Open People. 2. Ask someone you share no group with.
- **Expected:** 1: the note; only group-mates listed. 2: the refusal in plain words.
- **Edge / negative cases:** A company relay (the default) shows no note and everyone.
- **Automation:** Covered by: `relay::tests::group_calls_are_answered_by_request_id_and_fail_on_disconnect` (visibility) | Layer: unit | Gap: no test of the refusal against a real relay.
- **Needs relay:** yes
- **Status:** Shipped

## Gaps and planned (not testable yet)

- **GitHub-org teams** (create the team from an org, roles from org membership, auto-create a private `nandi-registry`): described in `docs/userguide/userguide.md` §9 and `docs/developer/github-auth.md`. They were removed in `91c8fad` (remove auth). Teams are any git URL, there is no GitHub API, and `commands/auth.rs` and `http/github.rs` no longer exist. Roles other than Owner (self, on create) and Member (on join) are never assigned.
- **Entra ID group teams and Microsoft sign-in** (Settings → Entra ID, device code, "Sync from Entra ID"): described in userguide §9 and `docs/developer/entra-setup.md`. Added in `d472f58` and later removed; nothing of it remains in the code (the old `entra_teams_cache` table is gone from the v2 schema).
- **Userguide §9 states team shares come from "GitHub orgs" and branch `nandi/share-…`**: the code pushes `nandi/share-<slug>-<agent>`. Docs to update.
- **Userguide §9 says delegation settings live in the agent's "Permissions" tab**: they are in Agent → Settings → Sharing.
- **Userguide §10 "Delegate: the Relay screen lists agents online … Call"**: the old Relay screen and its strings (`relay.agents.*`, `relay.call.*`) no longer render. Calling now happens from Library → Shared (TM-14).
- **LAN warning banner** ("Running on the local network. Public relay coming soon."), promised by `docs/developer/relay-defaults.md` and userguide §10: the `relay.lanBanner.*` strings exist, but no component renders them.
- **"Install locally" for a team agent**: the button is always disabled ("Installing a team agent locally isn't available yet", `SharedWithMe.tsx`).
- **No UI to join a team by ID**: `join_team` is API-only and only re-syncs a known row.
- **Delegation history page with filters** (status, capability, owner, tokens columns; `delegation.history.*` strings): there is no such page; delegations show in Home → All activity, and the history filters UI is not rendered.
- **The live public relay** (`wss://relay.vaultdb.ai/relay`): joining it is built (RL-45 to RL-47); the relay itself isn't deployed yet (DNS, TLS, the GitHub, Google and Microsoft OAuth apps). See `docs/developer/relay-defaults.md` "The public relay".
- **Relay token UX**: the token flow exists in nandi-relay's `src/server.rs`, but there is no way to issue or enter tokens other than by hand (`relay-defaults.md`).
- **Relay downloads and release packaging** (`nandi-relay` zip/tar.gz, container image, `--version` flag): planned. The relay is built in nandi-relay, which has no release workflow yet.
- **`.nandi/llm.yaml` `routing.fallbacks`**: validated and shown, but not used for routing (`network-and-reassignment.md`).
- **Error messages naming instance IDs and evidence latency counting cancelled or reassigned attempts**: open items.
- **Network isolation for agent containers** (enforced policies wave 1, part two): `docs/tasks-todo/task-x-enforced-policies.md`. This belongs to the sandbox area, but it affects what delegated tasks can reach.
