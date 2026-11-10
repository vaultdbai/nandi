# Task: Close the container gaps, then make policies enforced

**From:** design talk 2026-10-21. The owner proposed IAM-style roles. We chose to build on the existing `.nandi/policies/` files instead: one place to say what agents may do, aimed at one agent, a group, or everything.
**Size:** L, in four waves. Wave 1 comes first and ships on its own. A policy is only as strong as the container it limits.
**Out of scope:** budgets (tokens or money per policy). They come later and will reuse the trial's reserve-and-settle code in `services/llm_trial.rs`.
**Docs to update:** `docs/developer/egress.md`, `agent-runtime.md`, `harness-container.md`, `runtime-routing.md`, `data-model.md`, `domain-model.md`, `resources/studio-guides/approvals-and-policies.md`, the agents page design (drop the "Approvals aren't enforced" note).

## Why

Today each agent's `skill.md` states its own permissions (`services/domain/capability.rs`: `ToolPolicy`, `NetworkPolicy`, `AwsPolicy`). The local user is the only authority. `.nandi/policies/*.md` are prose: Studio and the ask box read them, and nothing enforces them (`services/repos/policy_repo.rs`).

The enforcement underneath also has holes:

| Gap                                      | Where                                                                                              | Effect                                                                                                     |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Containers on the default bridge network | `runtime.rs` (no `network_mode`), `harness/adapter.rs` (`--network` only with the LiteLLM gateway) | A process that ignores `HTTPS_PROXY` reaches the internet directly. `network.allow` is advisory.           |
| Harness ignores `network.allow`          | `harness/adapter.rs`                                                                               | No egress proxy at all for claude-code / codex / gemini-cli containers                                     |
| Raw keys in harness containers           | `harness/adapter.rs` env names, `harness/claude_code.rs`                                           | `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `GEMINI_API_KEY`, `AWS_*` readable by agent code           |
| Harness approvals off                    | `harness/acp.rs` `PermissionPolicy::AllowAll` default; `codex-config.toml` `danger-full-access`    | `requires_approval` does nothing for CLI runtimes                                                          |
| Writable root filesystem, user not set   | `runtime.rs`, `harness/adapter.rs`                                                                 | Agent can change its image's files; uid depends on the Dockerfile only                                     |
| Host placement                           | `domain/agent.rs` `RuntimePlacement::Host`, `adapter.rs` host spawn                                | Agent CLI runs on the host with no sandbox, against the product rule that agents run only in the container |
| Stdio MCP connectors on the host         | `services/connectors/mcp.rs` `spawn`                                                               | Connector code runs unsandboxed with its secrets as env                                                    |

## Design rules (apply to every wave)

1. **Policies only restrict.** `skill.md` says what an agent _needs_. Policies say what it's _allowed_. The agent gets the overlap. A deny anywhere wins. Adding a policy, even on `*`, can never widen access. This matches the plugin-packs rule that pack policies only add restrictions.
2. **Enforce where the agent can't reach:** the orchestrator, container settings, the egress proxy and the LLM proxy. Never in files or env inside the container.
3. **Fail closed.** If a policy file doesn't parse, or the container network or proxy can't start, the affected agents don't run. Say why in plain words.
4. **Gold Copy.** Policies stay files in `.nandi/policies/`, written only by `PolicyRepository`. Any SQLite projection can be rebuilt.
5. **Only the user loosens.** Studio may propose a looser policy, but only the user applies it, after a before-and-after view. Tightening may still go through the normal propose-then-apply flow.

## Wave 1: close the container gaps (done)

Shipped in #8 and #9: read-only, non-root containers (`org.nandi.sandbox=3`); no `placement: host`; coding agents ask before they act (`harness/approvals.rs`); every agent container on the internal `nandi-agents` network, whose only exit is the `nandi-egress` relay to the egress proxy; Nandi's tools over `docker exec` stdio; leak tests in the sandbox suite (`sandbox_suite.rs`, run in CI since 2026-11-09).

Two items moved to the policy waves, because a policy is what configures them:

- Keys in coding-agent containers → **Wave 3, model access through the gateway**.
- Stdio connectors on the host → **Wave 2 `connectors`** and **Wave 3, connectors in containers**.

## Wave 2: policy format and evaluator

```markdown
---
id: finance-safe
applies_to: [agent:report-*, workflow:month-end] # or ["*"]
tools: { deny: [shell], ask: [write_file] }
files: [{ path: 'D:/Finance', mode: read }]
network: { allow: ['*.amazonaws.com:443'] }
aws: { services: [{ service: s3, actions: ['Get*', 'List*'] }] }
models: { allow: ['claude:*'] }
connectors: { allow: [github, filesystem], run_on_host: [] }
runtime: { placement: container }
---

Why this exists, in plain words. The body never grants anything; Studio and the ask box still read it as context.
```

- [x] `services/domain/policy.rs`: a pure `Policy` type parsed from the frontmatter. Validation rejects unknown keys, bad patterns, and relative paths in `files`. A `files` entry is a path or a shared-folder id, with `mode: read | write`. Files with no frontmatter rules stay prose-only and apply nothing, so today's files keep working.
- [x] Targets in `applies_to`:
  - `agent:<id or glob>`
  - `workflow:<id or glob>`
  - `tag:<tag>` (add `tags` to `skill.md` frontmatter)
  - `*`
- [x] `effective_permissions(agent, policies) -> EffectivePermissions` (done as `domain::policy::effective` / `PolicySet::effective`):
  - starts from what `skill.md` asks for;
  - intersects with each policy that applies;
  - lets deny beat ask beat allow;
  - records which policy decided each item, for the UI.
  - Pure and fully unit-tested: overlaps, globs, deny-wins, empty policy set, and `*` plus a specific policy.
- [x] **Evaluator choice.** Because policies only restrict, the logic is set intersection plus pattern matching, so a small built-in evaluator in `services/domain/` is enough for now. Keep Cedar (`cedar-policy`) as the option if central management in v2 needs a shared policy language.
- [x] Load policies through `PolicyRepository` (`load_all`; Studio validates with the same parser). Notes keys (`name`, …) are allowed at the top level; the rule blocks reject unknown keys. A file that doesn't parse blocks the agents it might target (rule 3) and shows as an error on the policy screen.
- [ ] Workflow steps (the evaluator takes the workflow in `Subject`; wave 3 passes it): a workflow's policies also apply to every agent while it runs as a step of that workflow.
- [x] `connectors`: `allow` narrows the connectors an agent's `skill.md` names (`connectors: [..]`); `run_on_host` lists trusted connectors allowed to run on the host instead of in a container. Restrict-only like the rest: a connector runs on the host only if every applying policy that mentions it allows that, and an agent with no policy keeps today's behaviour until Wave 3 flips the default.
- [x] `models`: patterns over `provider:model` (`claude:*`, `gpt:gpt-5*`). Applies to Nandi's own runs and to coding agents' CLI models.

## Wave 3: wire enforcement

Each check reads `EffectivePermissions` and never reads the raw `skill.md` fields. **Done this way:** `RepoStore::skill` runs every `SkillConfig` through `services::policies::narrow`, so everything that reads it (orchestrator, egress proxy, container mounts, the harness router) gets the narrowed result, and a broken policy or a refused model stops the agent there.

- [x] **Tools and approvals:** the orchestrator's `allows_tool` and approval flag (`orchestrator.rs`) for native runs, and the ACP permission bridge from Wave 1 for harness runs.
- [x] **Connector tools:** (denied connector tools aren't offered or run; `ask` adds them to approvals) today they're checked before `allows_tool` (`orchestrator.rs`), so `tools.deny` doesn't reach them. Make `deny` and `ask` apply to `<connector>__<tool>` names too.
- [x] **Network and AWS:** the egress proxy enforces the effective allow-list and AWS grants, not the agent's own.
- [ ] **Model access through the gateway** (was Wave 1 "no raw keys"). Coding agents' model calls go through the LiteLLM gateway for every provider, not only Bedrock: the container gets a per-run key that works only there, and the real Anthropic, OpenAI, Gemini and AWS credentials stay on the host (`harness/gateway.rs`).
  - **Done without the gateway:** `models.allow` is enforced for Nandi's own runs (`services::policies::narrow`) and for coding agents' CLI models before their run starts (`runtime_router::plan`, `cli_model_ref`). Still open here: keys out of containers, which needs every provider through LiteLLM and real keys to verify.
  - It's also where per-policy budgets attach later (see Later).
  - Open: a Claude subscription sign-in (`CLAUDE_CODE_OAUTH_TOKEN`) may not work through a gateway. Test it; if it can't, keep that one in the container and say so in the UI.
- [x] **Connectors in containers** (was Wave 1 "stdio connectors"). **Done:** the stdio server runs in the `nandi-harness` image (Node, Python, uv), hardened like agents, on `nandi-agents`, with egress identity `connector:<id>` (package registries + `config.network`); `run_on_host` keeps a separate host instance (`<id>#host`) for the agents whose policy lists it. Verified by `connector_container_e2e`. Stdio MCP connectors run in their own container on `nandi-agents`, with only the secrets in their `secret_refs` and the network their policy allows, like Docker's MCP Gateway (`services/connectors/mcp.rs`).
  - A connector in an applying policy's `run_on_host` runs on the host as today. That's for the ones that need the desktop.
  - Pilots stay on the host: they control the computer, and keep their per-capability grants.
  - Connectors not allowed by policy aren't started for that agent.
- [x] **Files:** shared folders already exist (`services/connectors/folders.rs`, `66760e3`). Each folder has `access: read | write`, agents name theirs in `skill.md`, and folders mount at `/agent/folders/<id>` for native and CLI runtimes. Policies narrow that list and add no mounts of their own:
  - a folder that no applying policy allows is dropped (no tool access, no mount);
  - `mode: read` turns a write folder into a read-only one, for both the tools and the `:ro` mount.
  - The domain check in Wave 2 matches `files` paths against the folder's `path`, so `files` can also name folder ids: `{ folder: reports, mode: read }`.
- [x] **Pilots:** `pilots.allow` narrows the pilots an agent's tools come from and refuses calls to the rest. (Placement no longer exists.)
- [x] **Audit:** log each deny and each approval decision to the audit log (`audit.rs`) with the policy id. Add a `decided_by` column to `approvals` (local user for now). **Done:** the decisions files in the gold copy (`.nandi/decisions/<yyyy-mm>.md`, Home › Decisions; `decisions.log` was folded into them on 2026-11-09); schema 2, and an older `state.db` is refused with what to delete.

## Wave 4: screens

- [ ] **Policy screen:**
  - A form for the frontmatter rules: targets, tools, files, network, models, runtime. **Done in part (#12):** Studio's policy form has targets and the tools, models, connectors and pilots lists; network and files are still YAML only.
  - The Markdown body stays editable beside it.
  - A preview lists the agents and workflows the policy reaches and what changes for each.
- [x] **Agent page:**
  - A "Permissions" panel showing what the agent asks for, what it actually gets, and which policy decided each line. **Done (#12):** Settings › Permissions starts with the policies that apply and what each changed (`AgentPolicies.tsx`, `agent_policies`).
  - Replaces the current permissions line (the Library page design).
- [ ] **Blocked log:** recent denies per agent, so the user can see why something failed and add an allow if they want to.
- [ ] **Studio:**
  - [x] Update its guide (`approvals-and-policies.md`) so it drafts policies in the new format (#12).
  - Any looser change shows the before-and-after view from rule 5.
- [x] **Locale strings** in `/locales/*.json`.

## Later (not in this task)

- **Budgets** per policy (tokens or money, per agent or shared, a warning before the hard stop). Enforced at the gateway once Wave 3 routes every model call through it.
- **Managed policies:** a second, read-only policy source (a system folder or MDM, later a central server). It's merged with the same rules and shown as "Managed by your organization", with an admin choice of which values users may change.
- **Roles** as named bundles: a policy whose `applies_to` lists agents, chosen by name on the agent page.
