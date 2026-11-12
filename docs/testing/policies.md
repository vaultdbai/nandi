# Policies

Policy files in `~/.nandi/repo/.nandi/policies/<id>.md` say what agents may do. They are aimed at one agent, a group or everything, and they can only take permissions away. The app parses these files and works out what each agent ends up with (PO-01 to PO-10), applies the result to every run (PO-13), and shows it on the agent page (PO-11); Studio has a form for the rules (PO-12). See `docs/developer/sandbox-test-checklist.md` sections 5 and 6 for checking it by hand. Code roots: `src-tauri/src/services/domain/policy.rs` (pure: `Policy`, `PolicySet`, `effective`), `src-tauri/src/services/policies.rs` (`narrow`, `view`), `src-tauri/src/services/repos/policy_repo.rs` (`PolicyRepository::load_all`, `read`, `save`), `src-tauri/src/services/watcher.rs` (hand edits, PO-11), `src-tauri/src/services/studio/validate.rs`. Run the unit tests with `cargo test --manifest-path src-tauri/Cargo.toml --lib domain::policy` (`docs/developer/sandbox-test-checklist.md` 5.1).

## Policy files

### PO-01 — Policy files: prose and rule policies

- **What:** A policy is Markdown with YAML frontmatter. With `applies_to` and rule blocks (`tools`, `files`, `network`, `aws`, `models`, `connectors`, `pilots`), it is a rule policy. With no frontmatter, or none of the rule blocks, it is prose: Studio and the ask box read it, and it applies nothing, so policy files written before #10 keep working. Any other top-level key (`name`, `description`, `require_approval`, …) is a note: it stays in the file and enforces nothing. The body never grants anything.
- **Where:** `.nandi/policies/<id>.md`; `domain/policy.rs` (`Policy::parse`, `is_enforced`, `RULE_KEYS`); Ask (the Studio page) → **New** ▾ → **Policy** (AI-05) for editing. The Studio starter template (`studio/artifact_loader.rs`) is a prose policy (`name`, `description`, `applies_to: []`). Studio's policy form shows only enforced rules; other top-level keys load as notes.
- **Preconditions:** None.
- **Steps:** 1. In Ask, choose **New** ▾ → **Policy**, type `style` in **New ID**, click **Create**, and click **Save** without changing the template. 2. Save a policy whose body is only `# Be kind`. 3. Save one with `applies_to: ['*']` and `tools: { deny: [shell] }`.
- **Expected:** 1–2 save, are valid, and apply nothing (`is_enforced` false). 3 saves and is a rule policy. All three are files under `.nandi/policies/` and are not projected into SQLite.
- **Edge / negative cases:** `runtime: { placement: container }` (shown in the plan's example) is not a rule block, so it is kept as a note and enforces nothing; an empty frontmatter block is prose.
- **Automation:** Covered by: `domain/policy.rs` (`prose_policies_apply_nothing_and_notes_are_kept`) | Layer: unit | Gap: no test that the Studio template parses as a prose policy.
- **Needs relay:** no
- **Status:** Shipped

### PO-02 — Policy validation errors

- **What:** A policy that can't be enforced as written is refused with the policy id, the field and a plain reason:
  - rule blocks with no `applies_to`: "is missing, so its rules (tools, …) would apply to nobody. Add applies_to: ["*"] or name the agents";
  - a bad target: "has 'robot:x'; use \*, agent:<id> or tag:<tag>", or a bad glob;
  - a `workflow:` target: "has 'workflow:month-end', but workflow targets aren't supported yet: runs don't say which workflow they're a step of, so the rule would apply to nothing. Use \*, agent:<id> or tag:<tag>" (PO-04);
  - an unknown key inside a rule block (`tools: { block: [shell] }`);
  - a `files` entry with both `path` and `folder` or neither, or a relative path ("paths must be absolute (D:/Data, /home/me/data) or under ~");
  - a bad host pattern in `network.allow` (`a.*.com`), a bad glob in `models.allow`, or an `aws.services` entry without `service`.
- **Where:** `domain/policy.rs` (`Policy::parse`, `Target::parse`, `validate_files`, `validate_aws`, `validate_patterns`, `NetworkPolicy::normalized`).
- **Preconditions:** None.
- **Steps:** Validate each case above (PO-03 shows how), then the fixed version of each.
- **Expected:** Each bad case is invalid with the reason named; each fix is valid. `files` paths are normalized (backslashes to `/`, no trailing slash); `aws` service names are lowercased.
- **Edge / negative cases:** `~` and `~/x` count as absolute; `C:` alone means the drive (`C:/`); tool names are trimmed and empty ones dropped.
- **Automation:** Covered by: `domain/policy.rs` (`rules_without_a_target_are_refused`, `bad_rules_are_refused`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### PO-03 — Studio checks policies with the enforcing parser

- **What:** Studio's live validation for a policy uses the same parser the app will enforce with, so the editor shows "Not valid yet, so it can't be saved:" with the error and keeps **Save** disabled for a policy that would block agents.
- **Where:** Ask → **New** ▾ → **Policy** (`src/components/studio/ArtifactEditor.tsx`: Save disabled while `validation.ok` is false); `studio/validate.rs` (`ArtifactRef::Policy` → `domain::policy::Policy::parse`); command `validate_artifact`, HTTP `POST /api/studio/validate` with body `{"artifact":{"kind":"policy","id":"x"},"content":"..."}`.
- **Preconditions:** App or `nandi --server` running.
- **Steps:** Follow `docs/developer/sandbox-test-checklist.md` 5.2–5.4, typing each policy into the **Raw file** box of a new policy in Ask: 1. Validate a policy with `tools: { deny: [shell] }` and no `applies_to`. 2. Validate `applies_to: [robot:x]`, then `tools: { block: [shell] }`. 3. Validate an existing prose policy. 4. Repeat 1 over HTTP.
- **Expected:** 1–2 show "Not valid yet, so it can't be saved:" and "Not a valid policy: …" naming `applies_to`, the bad target or the unknown key; Save disabled. 3 valid; saves as before. 4 returns `ok: false` with the same error.
- **Edge / negative cases:** saving runs the same parser: `studio/artifact_saver.rs` refuses a policy `Policy::parse` refuses, and the Studio chat's save tool (`studio/save.rs`) validates policies before saving, like agents and workflows. Only a file written by hand can be broken (PO-10).
- **Automation:** Covered by: `studio/tests.rs` (a policy with rules and no `applies_to` isn't saved), `src/components/studio/Studio.test.tsx` | Layer: api | Gap: no test that validates a policy artifact through `validate_artifact`.
- **Needs relay:** no
- **Status:** Shipped

### PO-04 — Targets (`applies_to`)

- **What:** `applies_to` lists who the rules are for: `*` (every agent), `agent:<id or glob>` (`agent:report-*`) and `tag:<tag>` (agents whose `skill.md` lists the tag in `tags`). A policy applies when any of its targets matches. A `workflow:<id>` target is refused, with why, wherever a policy is saved or read (Studio, the Studio chat's save tool, a pack install, a group's policy, a file edited by hand, which then blocks the agents it may cover until fixed): runs don't carry their workflow yet, so such a rule would silently apply to nothing, and a rule the user thinks applies but doesn't is a trust bug. Studio's policy form says so under **Applies to**.
- **Where:** `domain/policy.rs` (`Target`, `Target::parse`, `Policy::applies`, `Subject { agent, tags }`); `src/components/studio/PolicyForm.tsx` (`studio.field.appliesToHint`).
- **Preconditions:** None (unit level).
- **Steps:** 1. Evaluate `applies_to: [agent:report-*, tag:finance]` for agent `report-weekly`, agent `writer` and `writer` with tag `finance`. 2. Evaluate `applies_to: ['*']` for any agent. 3. Save a policy with `applies_to: [workflow:month-end]` in Studio. 4. Open a policy in Studio and read under **Applies to**.
- **Expected:** 1. Applies to `report-weekly`, not to plain `writer`, applies to `writer` with the tag. 2. Applies. 3. Not saved: "…has 'workflow:month-end', but workflow targets aren't supported yet: runs don't say which workflow they're a step of, so the rule would apply to nothing. Use \*, agent:<id> or tag:<tag>". 4. The targets that work, and that workflow targets aren't supported yet.
- **Edge / negative cases:** tags match exactly (no globs); `tags:` entries are trimmed and empty ones dropped (`Agent::tags`); `['*', workflow:x]` is refused too (one bad target refuses the policy).
- **Automation:** Covered by: `domain/policy.rs` (`targets_pick_agents_and_tags`, `a_workflow_target_is_refused_with_why`), `services/policies.rs` (`tags_in_skill_md_select_policies`), `domain/agent.rs` (`every_frontmatter_key_is_documented` reads `tags`), `PolicyForm.test.tsx` (the hint) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped (`*`, `agent:` and `tag:`; `workflow:` refused until runs carry their workflow)

### PO-05 — Effective permissions: tools and approvals

- **What:** `tools.allow` keeps only the listed tools that `skill.md` already allows (it never adds one), `tools.deny` removes a tool whatever any other policy allows, and `tools.ask` adds the tool to the approval list. Deny beats ask, and ask beats allow. A policy never widens: an allow-all policy (`tools.allow` of everything, `network.allow: [{host: '*'}]`) changes nothing.
- **Where:** `domain/policy.rs` (`effective`, `apply`, `Effective::allows_tool`, `needs_approval`, `denied_tools`).
- **Preconditions:** None (unit level).
- **Steps:** 1. Evaluate policy `a-deny` (`tools: { deny: [shell] }`) with `b-allow` (`tools: { allow: [shell, read_file], ask: [read_file] }`) for an agent with no `allowed-tools` (every runtime tool) whose `write_file` asks first. 2. Swap the order. 3. Evaluate a wide policy for an agent with only `read_file`.
- **Expected:** 1. `shell` refused (denied by `a-deny`), `read_file` allowed and needs approval, `write_file` refused (outside the allow list) and still needs approval; `policies` = `[a-deny, b-allow]`. 2. Identical result. 3. Still only `read_file`; no changes recorded.
- **Edge / negative cases:** an agent with no `allowed-tools` (every runtime tool) gets exactly the policy's `allow` list; connector tools (`<connector>__<tool>`) are covered by `tools.deny` and `tools.ask` too.
- **Automation:** Covered by: `domain/policy.rs` (`deny_wins_and_ask_is_added`, `a_policy_never_widens`) | Layer: unit | Gap: none here; run-time use is PO-13.
- **Needs relay:** no
- **Status:** Shipped

### PO-06 — Effective permissions: files and shared folders

- **What:** `files` narrows the agent's shared folders. A folder that no `files` entry covers is dropped. A `mode: read` entry turns a write folder into a read-only one. Entries match a folder by host `path` (the folder or anything inside it) or by shared-folder id (`{ folder: reports }`). Policies add no folders of their own.
- **Where:** `domain/policy.rs` (`apply` files block, `within`, `FileRule`, `FileMode`, `FolderRequest`).
- **Preconditions:** None (unit level).
- **Steps:** 1. Agent asks for `reports` (`D:/Finance/reports`, write) and `photos` (`C:/Users/me/Pictures`, read); policy `files: [{ path: 'd:\finance', mode: read }]`. 2. Policy `files: [{ folder: photos }]`.
- **Expected:** 1. Only `reports`, now read-only (`Narrowed("read only")`); `photos` removed. 2. Only `photos`.
- **Edge / negative cases:** paths compare by folder, not by prefix (`D:/Finances` is not inside `D:/Finance`); Windows drive paths compare without case, other paths with case.
- **Automation:** Covered by: `domain/policy.rs` (`folders_are_dropped_or_made_read_only`, `paths_compare_by_folder_not_by_prefix`) | Layer: unit | Gap: none here; run-time use is PO-13.
- **Needs relay:** no
- **Status:** Shipped

### PO-07 — Effective permissions: network and AWS

- **What:** `network.allow` keeps each host the agent asks for only where a policy host also covers it. The result is the more specific of the two hosts, with the methods both allow, and the ports must match. `aws.services` keeps only the listed services and narrows their actions to the policy's patterns (case-insensitive, `Get*`). An empty `actions` list means any action.
- **Where:** `domain/policy.rs` (`intersect_host`, `covers`, `narrow_actions`, `AwsRules`).
- **Preconditions:** None (unit level).
- **Steps:** 1. Agent allows `*.github.com` (any method) and `example.com` (`GET`); policy allows `{ host: api.github.com, methods: [GET, POST] }` and `{ host: '*.example.com' }`. 2. Agent has S3 `GetObject`, `PutObject` and EC2 (any action); policy `aws: { services: [{ service: S3, actions: ['get*'] }] }`.
- **Expected:** 1. `api.github.com` with `GET, POST` (the more specific host; `Narrowed`); `example.com` removed (`*.example.com` covers subdomains only). 2. Only S3 `GetObject`; EC2 removed.
- **Edge / negative cases:** `*.x.com` covers `*.a.x.com` but not `x.com`; `a.com:8443` vs `a.com` don't intersect; no methods in common removes the host.
- **Automation:** Covered by: `domain/policy.rs` (`hosts_narrow_to_the_more_specific_and_shared_methods`, `aws_grants_keep_only_allowed_services_and_actions`) | Layer: unit | Gap: none here; the egress proxy reads the narrowed grants (PO-13).
- **Needs relay:** no
- **Status:** Shipped

### PO-08 — Effective permissions: models

- **What:** `models.allow` is a list of `provider:model` patterns (`anthropic:*`, `openai:gpt-5*`). A model must match every applying policy's list. `allows_model` names the policy that refuses it. With no `models` rules, any model is allowed.
- **Where:** `domain/policy.rs` (`Effective::allows_model`, `model_rules`).
- **Preconditions:** None (unit level).
- **Steps:** 1. Policy `only-anthropic` with `models: { allow: ['anthropic:*'] }`; agent asks for `openai:gpt-5`. 2. Check `anthropic:claude-sonnet-5`. 3. No policies, check `x:y`.
- **Expected:** 1. Refused by `only-anthropic`; change `model openai:gpt-5` recorded. 2. Allowed. 3. Allowed.
- **Edge / negative cases:** patterns match Nandi's own model strings (`claude:*`, `gpt:*`, `deepseek:*`, `ollama:*`; MD-04).
- **Automation:** Covered by: `domain/policy.rs` (`models_must_match_every_applying_policy`) | Layer: unit | Gap: the LLM proxy and the LiteLLM gateway don't refuse models themselves; the check is before the run (PO-13).
- **Needs relay:** no
- **Status:** Shipped (Nandi's own runs: the agent's model; coding agents: the CLI's model as `claude:`/`gpt:`/`gemini:<model>`, checked before the run starts)

### PO-09 — Effective permissions: connectors, pilots and `run_on_host`

- **What:** `connectors.allow` keeps only the listed connectors the agent names, and `pilots.allow` does the same for pilots. `allow: []` removes all. `connectors.run_on_host` is the one widening: it marks trusted connectors to run on the host instead of in a container. That holds only for connectors still allowed after every applying policy, so another policy that drops the connector also takes back its host run.
- **Where:** `domain/policy.rs` (`ConnectorRules`, `AllowList`, `connectors_on_host`).
- **Preconditions:** None (unit level).
- **Steps:** 1. Policy `conn` (`*`): `connectors: { allow: [github, notes], run_on_host: [notes, gone] }`, `pilots: { allow: [] }`; evaluate for agent `a`. 2. Add policy `gh` (`agent:b`, `connectors: { allow: [github] }`) and evaluate for agent `b`.
- **Expected:** 1. Connectors `github`, `notes`; on host `notes` only (`gone` isn't asked for); pilot `browser` removed. 2. Connectors `github` only; nothing on the host.
- **Edge / negative cases:** with no `run_on_host`, stdio connectors run in their own container (Wave 3b); `run_on_host` gives the agents it covers a host instance instead.
- **Automation:** Covered by: `domain/policy.rs` (`connectors_and_pilots_narrow_and_trusted_ones_run_on_the_host`) | Layer: unit | Gap: none here; where connectors run is CP-22.
- **Needs relay:** no
- **Status:** Shipped

### PO-10 — Broken policy files block the agents they may cover

- **What:** Loading the folder (`PolicyRepository::load_all`) parses every `<id>.md` and keeps broken files apart (`PolicySet.broken`). A broken file still blocks (fail closed). It blocks the agents its `applies_to` names when that much can be read, and every agent when it can't. `PolicySet::effective` then answers "The policy '<id>' has an error, so agents it may cover don't run until it's fixed: <error>". No folder means no policies.
- **Where:** `domain/policy.rs` (`PolicySet::parse`, `blocking`, `effective`, `BrokenPolicy`, `readable_targets`), `repos/policy_repo.rs` (`load_all`), enforced through `RepoStore::skill` → `services::policies::narrow`.
- **Preconditions:** None (unit level).
- **Steps:** 1. Policies `ok` (`*`, deny `shell`) and `named` (`agent:report-*`, `tools: { block: [x] }`). Evaluate `writer` and `report-weekly`. 2. A file whose frontmatter is `applies_to: [` (unreadable); evaluate anyone. 3. By hand, put a broken file in `.nandi/policies/` (checklist 5.5).
- **Expected:** 1. `writer` runs with `shell` denied; `report-weekly` is blocked with an error naming `'named'`. 2. Blocked. 3. The agents it names don't run: the run fails with that message (native and coding agents alike).
- **Edge / negative cases:** non-`.md` files in the folder are skipped; an unreadable file (I/O error) fails the whole load.
- **Automation:** Covered by: `domain/policy.rs` (`a_broken_policy_blocks_whom_it_names_or_everyone`), `services/watcher.rs` (`policy_edits_on_disk_are_checked_and_published`: a broken file on disk stops the agent, deleting it lets it run), `http/policies.rs` (`an_agents_policies_say_what_they_change_or_why_it_cannot_run`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Policies at run time and on screen

### PO-11 — Agent page: the policies over an agent

- **What:** Agent page › Settings › Permissions starts with the policies that apply to the agent: "No policy applies to this agent: it can do what its skill.md says.", or "Policies that apply: <ids>" and a list "What the policies changed", one row per change with the item (`tool shell`, `folder reports`, `host example.com`), a badge ("Removed", "Narrowed to <detail>", "Asks first", "Runs on this computer") and "by <policy>". With policies that change nothing: "They don't change anything this agent asks for." When the agent can't run (a broken policy, a refused model) the section is an alert: "This agent can't run: <reason>". The section refreshes when the agent's files change, when a policy file changes (saved in Studio or by Ask's chat, or edited by hand: the repo watcher, DT-13), and on resync.
- **Where:** Agent page → Settings → Permissions. `src/components/agents/AgentPolicies.tsx`, `src/hooks/useAgentPolicies.ts` (`agentPoliciesQueryKeys`), `src/lib/agent-sync.ts` (`repo://file_changed` for `kind: agent` and `kind: policy`, `studio://artifact_saved`), `services/watcher.rs` (`Target::Policy`); command `agent_policies` (`commands/policies.rs`), HTTP `POST /api/rpc/agent_policies` `{agentId}` (`http/policies.rs`); `services/policies.rs` (`view`, `agent_policies`, `AgentPolicies`, `PolicyChange`, `ChangeOutcome`), `RepoStore::declared_skill`.
- **Preconditions:** An agent `writer` that allows `example.com` in `network.allow`.
- **Steps:** (checklist 6.12) 1. No policies: open the agent page › Settings and look at Permissions. 2. With the agent page open, save policy `safe` in Ask (**New** ▾ → **Policy**, `applies_to: ['*']`, `tools: { deny: [shell], ask: [read_file] }`, `network: { allow: [{ host: api.github.com }] }`). 3. `GET /api/agents/writer/policies`. 4. With the page open, write a broken policy `oops` (`tools: { nope: 1 }`) by hand. 5. Delete `oops.md`.
- **Expected:** 1. "No policy applies…". 2. Without a reload: "Policies that apply: safe"; rows `tool shell` Removed, `tool read_file` Asks first, `host example.com` Removed, each "by safe". 3. JSON `{policies: ["safe"], blocked: null, changes: [{item, outcome, detail, by}]}` with outcomes `removed`, `narrowed`, `mustAsk`, `onHost`. 4. Within about a second, without touching the agent: alert "This agent can't run: The policy 'oops' has an error…", and `repo://validation_failed` (`kind: policy`) plus `repo://file_changed` (`result: invalid`) on the WS stream. 5. The alert goes and the policies show again (`repo://file_changed`, `result: changed`).
- **Edge / negative cases:** a bad agent id is refused ("Invalid agent id '<id>'"), an unknown one with "Agent '<id>' not found" (HTTP 400); a broken policy isn't kept at its last good version like other files: it blocks the agents it may cover until it's fixed (PO-10).
- **Automation:** Covered by: `src/components/agents/AgentPolicies.test.tsx` (4), `services/policies.rs` (`the_view_says_what_each_policy_changed_or_why_it_is_blocked`), `src/lib/agent-sync.wave1.test.ts` ("invalidates git status when a repo file changes", "refreshes every agent’s policies when a policy file changes", "refreshes every agent’s policies when Studio saves a policy"), `http/policies.rs` (`an_agents_policies_say_what_they_change_or_why_it_cannot_run`), `services/watcher.rs` (`policy_edits_on_disk_are_checked_and_published`), `studio/tests.rs` (`a_policy_saved_from_the_chat_is_announced`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### PO-12 — Studio policy form and agent tags

- **What:** Studio's Policy form (in Ask) edits the rule lists as comma-separated lists, after "Name", "Description" and "Applies to": "Only these tools", "Never these tools", "Ask before these tools" (`tools.allow`, `deny`, `ask`), "Only these models (e.g. claude:\*)" (`models.allow`), "Only these connectors" and "Connectors that run on this computer" (`connectors.allow`, `run_on_host`), "Only these pilots" (`pilots.allow`). `network` and `files` are lists of objects, edited in the YAML. The Agent form has "Tags (policies can aim at them)" (`tags`, PO-04). Ask's guide (`approvals-and-policies.md`) drafts policies in this format.
- **Where:** Ask → **New** ▾ → **Policy**, and the **Agent** tab of the right pane (pick the agent in its dropdown). `src/components/studio/PolicyForm.tsx`, `AgentForm.tsx`, `form-fields.tsx` (`ListField`); `src-tauri/resources/studio-guides/approvals-and-policies.md`; `domain/policy.rs` (`POLICY_KEYS`).
- **Preconditions:** App running.
- **Steps:** 1. **New** ▾ → **Policy**, New ID `no-shell`, **Create**; in **Fields** set Applies to `*`, Never these tools `shell, write_file`; **Save**. 2. Open a policy whose YAML has `models: { allow: ['claude:*'] }` (pick it in the Policy dropdown, which lists every policy file by name; a broken one is marked "(has errors)"). 3. In the Agent tab, pick an agent, set "Tags (policies can aim at them)" to `finance` and **Save**; aim a policy at `tag:finance`; open the agent's Permissions (PO-11).
- **Expected:** 1. The file gets `tools: { deny: [shell, write_file] }` and keeps `applies_to: ['*']`; it validates (PO-03). 2. The models field shows `claude:*`. 3. skill.md has `tags: [finance]`, and the policy shows as applying.
- **Edge / negative cases:** there is no max-cost or require-approval field: such keys typed into the YAML stay as notes that enforce nothing (PO-01); the `gateway` entry (NANDI.md) shows only the gateway fields ("Display name", "Max tasks at a time", "Requests per hour", "Approval needed for").
- **Automation:** Covered by: `src/components/studio/PolicyForm.test.tsx` ("writes rule lists into the frontmatter", "shows the rules a file already has"), `studio/guides.rs` tests (guide keys are real names) | Layer: unit | Gap: no test of the Agent form's Tags field.
- **Needs relay:** no
- **Status:** Shipped

### PO-13 — Policies applied to every run

- **What:** `RepoStore::skill` sends every agent's config through `services::policies::narrow`, so everything that reads it gets what the agent may do after its policies:
  - the orchestrator: tools offered and run, approvals, connector tools and pilot tools (a forced call says "a policy doesn't allow it" / "a policy doesn't allow this pilot");
  - coding agents over ACP: `PermissionPolicy::Except` rejects a denied kind even with no `allowed-tools`, and `ask` adds approvals;
  - the egress proxy: the narrowed `network.allow` and AWS grants;
  - container mounts: folders dropped or mounted `:ro`.
    A native agent whose model a policy refuses doesn't run ("The policy '<id>' doesn't allow the model <model> for agent '<id>'"). A coding agent's CLI model (`claude:`, `gpt:` or `gemini:<model>`, `<provider>:default` when unset) is checked in `runtime_router::plan` before its container starts, and `fallback: native` doesn't get around it. A broken policy that may cover the agent stops it (PO-10). `~` in policy paths is the user's home.
- **Where:** `src-tauri/src/services/agents.rs` (`RepoStore::skill`, `policies`, `declared_skill`), `services/policies.rs` (`narrow`, `evaluate`, `apply`), `services/orchestrator.rs` (`declared_connectors`, tool and pilot checks), `services/orchestrator/runtime_router.rs` (`plan`, `cli_model_ref`, `ToolGrant`), `services/harness/acp.rs` (`PermissionPolicy::from_tools`), `services/egress/mod.rs` (`start_server` capabilities), `runtime.rs` (`folder_mounts`).
- **Preconditions:** Docker and both images (RT-01); a native agent and a Claude Code agent.
- **Steps:** Follow `docs/developer/sandbox-test-checklist.md` section 6 (6.1–6.11).
- **Expected:** As in the checklist: denied tools aren't offered and forced calls are refused; `ask` tools need approval; the proxy refuses hosts outside the narrowed list; the folder is read-only or not mounted; a refused model fails the run with the message above.
- **Edge / negative cases:** no policies leave skill.md as it is; the agent page's Permissions panel (AG-17) and Studio show the agent's own files, and PO-11 shows what the policies change.
- **Automation:** Covered by: `services/policies.rs` (`no_policies_leave_skill_md_as_it_is`, `policies_narrow_tools_network_and_folders`, `tags_in_skill_md_select_policies`, `a_refused_model_or_a_broken_policy_stops_the_agent`), `runtime_router.rs` (`a_policy_s_tools_reach_the_cli`, `a_policy_s_models_allow_covers_the_cli_model`), `acp.rs` (`permissions_follow_the_policy_and_the_approvals`), `domain/policy.rs` (`home_in_policy_paths_is_the_user_s`), `orchestrator.rs` (`a_tool_or_pilot_a_policy_denies_is_refused_not_run`: refused, and no container starts) | Layer: unit | Gap: checklist section 6 is manual.
- **Needs relay:** no
- **Status:** Shipped

### PO-14 — The decision record: who decided, and what a policy refused

- **What:** Every decision is one entry in this month's file in the gold copy, `.nandi/decisions/<yyyy-mm>.md` (the only record; there is no `decisions.log`): when, **Decided by**, the **Item** (kind, id and what it was about), the **Decision**, the reason or note, and the Ask chat it came from. Decided by is the local user's display name from NANDI.md (else their slug, else "you") for a person's decision, from a button or an Inbox Ask chat; `auto-approve rule (<rule>)` or `group policies` for work a rule or the groups let through; `policy <id>` for a call a policy refused (kind `policy`: `tool shell`, `pilot browser`, `browser controls`, a connector it forbids). A tool a policy denies is refused before anything else, even one in the agent's approval list (shell is, by default): no approval card, no approval row, one entry by the policy. An access request's entry names what it wanted (`triager: read with Issue tracker`) and the choice (`approved (allowSaved)`, `refused (deny)`); grants given or removed (Settings, an agent's Connectors panel, connecting or signing out of an account, removing a connector) are kind `connectorGrant`. An approval that expired isn't a decision: its status stays on the approval and in the run's history. The `approvals` table keeps `decided_by` too. Native and coding-agent approvals alike. Home › Decisions reads the files back.
- **Where:** `services/inbox/audit.rs` (`record` for a person, `record_in` for a rule or policy, `Decided::by`), `repos::decision_repo` (the only writer), `domain::decision` (format); callers `chat::approve_action` (`decider`, `db::set_approval_decider`; `approval_subject`), `policies::record_refused` (from `orchestrator.rs` for a tool or pilot, `pilots/browser.rs` for browser controls), `access::decide` (a connector a policy forbids), `grants::record`, `intake::gate_task` and `approve_as` (rules and groups). Home › Decisions (`DecisionsView`).
- **Preconditions:** None (the API level; Docker only for the policy steps with real tools).
- **Steps:** 1. Set your display name (`display_name` in NANDI.md, or Ask → Policy → `gateway` → "Display name"). Approve one `shell` call; refuse a connector access request. 2. A policy `tools: { deny: [shell] }`; make the model call `shell` anyway. 3. An auto-approve rule for a teammate; let their clean request in. 4. Open Home › Decisions, and the month's file in the repo.
- **Expected:** 1. Entries `approved` and `refused (deny)`, Decided by your name, the items named (`approver: shell`, `triager: read with Issue tracker`). 2. No approval card appears though `shell` needs approval; one entry: kind `policy`, `refused`, Decided by `policy <id>`, item `<agent>: tool shell`. 3. `approved`, Decided by `auto-approve rule (requests from <name>)`. 4. All of them, newest first.
- **Edge / negative cases:** a database from schema 1 is refused at start with "The local database is from another Nandi build… delete ~/.nandi/local/state.db…" (no migrations; agents and workflows live in `~/.nandi/repo` and aren't touched). Egress denials stay in `egress.log` without a policy id (the proxy doesn't know which policy removed a host). A second decision on a decided approval is refused and adds no entry. The Chief of Staff's confirm card in an Ask chat is not an entry of its own; the decision it confirms is, with its chat.
- **Automation:** Covered by: `approvals_suite.rs` (`approvals_and_refusals_are_on_the_record`, `a_denied_tool_is_refused_without_asking_first`: denied and needing approval, refused with no approval row and one entry by the policy, `a_decision_in_an_inbox_chat_is_recorded_with_the_chat`, `auto_approvals_are_recorded_as_such`: the month's file read back for each), `inbox/audit.rs`, `inbox/tool/tests.rs`, `db.rs` (`a_database_from_another_schema_is_refused_with_what_to_do`, the approvals test's decider round trip) | Layer: api | Gap: the Decisions page against real files is `ui-e2e`.
- **Needs relay:** no
- **Status:** Shipped

### PO-15 — Browser controls a policy refuses (`browser.refuse`)

- **What:** A policy can refuse browser controls: `browser: { refuse: [{ controls: [post, like], kinds: [submit], sites: [x.com], except: [start a post], say: … }] }`. Before a call that acts on the page (click, type, key, form, dialog, drag, upload), Nandi asks the page for the control's role and name (`browser_snapshot {target, depth: 1}`, else the tab list for the URL). A rule refuses the call when the page is on its sites (subdomains included; none = everywhere) and the control's name says one of `controls` (whole words; fields such as textboxes are never controls) or the call is one of `kinds` (Enter and Ctrl+Enter are submits). Only the page's own name can match `except`. The refusal is a tool error with the policy's id and `say`, recorded in the decisions file (kind `policy`, decided by `policy <id>`) as `browser controls`. A call that would be refused on the page the browser last showed isn't asked about first. The agent page shows "browser controls: refuses …".
- **Where:** `services/domain/browser_rule.rs`, `domain/policy.rs` (`Rules.browser`, `Effective.browser_refusals`), `services/pilots/refusal.rs`, `browser.rs` (`call`, `inspect`, `refused_before`), `tools.rs` (`requires_approval`).
- **Preconditions:** An agent with the browser pilot; a policy with `browser.refuse` that applies to it.
- **Steps:** 1. Have the agent click a refused button. 2. Press Ctrl+Enter on a refused site. 3. Click an `except` control. 4. Describe a refused button with the `except` words.
- **Expected:** 1-2. Refused with the policy's words, no approval asked, a `refused` entry in this month's decisions file, decided by the policy. 3. Runs. 4. Still refused.
- **Edge / negative cases:** An item with neither controls nor kinds, an unknown kind or a URL as a site is a broken policy (blocks the agents it names). Off the listed sites a "Post" button is not refused.
- **Automation:** Covered by: Rust `domain/browser_rule.rs` (5), `domain/policy.rs` `browser_refusals_add_up_and_keep_the_pilot`, `pilots/refusal.rs` (per site: LinkedIn, X, Facebook, Instagram, YouTube, engagement everywhere), `pilots/tools.rs` `a_click_a_policy_refuses_is_refused_not_asked_about` | Layer: unit | Gap: live sites (owner run).
- **Needs relay:** no
- **Status:** Shipped

### PO-16 — Policies a starter pack ships for its own agents

- **What:** A pack may carry `policies/<id>.md`. The installer checks each parses, has rules, and names only the pack's own agents exactly (`applies_to: [agent:<id>]`; no `*`, tags, patterns or other agents), else the install is refused. Installed, they apply as `<pack>/<id>` from the vendored folder, read-only, and aren't listed for editing.
- **Where:** `services/marketplace/install.rs` (`is_pack_policy`, `validate_pack_policies`), `domain/policy.rs` `Policy::only_for`, `services/repos/policy_repo.rs` (`load_all`, `pack_files`).
- **Preconditions:** A pack with a policy (Nandi Social).
- **Steps:** Install it; open the agent's Settings › Permissions.
- **Expected:** "Policies applied: social-pack/social-posting" and its change.
- **Edge / negative cases:** A pack policy for `*` or another agent: the install fails with the reason. Half-written installs (staging folders) are ignored.
- **Automation:** Covered by: Rust `install.rs` `a_pack_ships_policies_for_its_own_agents_only`, `policy_repo.rs` `installed_packs_policies_apply_but_are_not_listed_for_editing` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Gaps and planned (not testable yet)

From `docs/tasks-todo/task-x-enforced-policies.md`:

- Workflow steps: a workflow's policies applying to every agent while it runs as a step of that workflow. Until runs carry their workflow (and the egress proxy can tell runs apart), a `workflow:` target is refused (PO-04).
- Model access through the gateway: every coding agent's model calls through LiteLLM with a per-run key, real keys kept on the host; then `models.allow` refused at the gateway and the LLM proxy as well as before the run (PO-08).
- Runtime and pilot use deniable by policy (`runtime: { placement }` is a note today, PO-01).
- Wave 4 leftovers: a preview of the agents and workflows a policy reaches; a blocked log of recent denies per agent; the before-and-after view for any looser change; `network` and `files` in the policy form (YAML only today, PO-12).
- Later: budgets per policy (modelled on the trial gateway's reserve-and-settle), managed read-only policy sources ("Managed by your organization"), roles as named policy bundles.
