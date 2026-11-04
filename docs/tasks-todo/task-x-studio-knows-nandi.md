# Task: Studio knows Nandi, not just what's in it

**From:** architecture review 2026-10-20 (how Studio answers "build me an agent" / "run X" / "how do I…").
**Size:** M, in two waves. Wave A ships on its own; Wave B after it.
**Depends on:** nothing new.

## Why

Studio's system prompt (`services/studio/prompt.rs`) is generated from code, and its read-only `nandi_*` tools (`services/studio/tools.rs`) show everything this Nandi has right now. Studio can see the current state and validate files (`nandi_check_artifact`), but it lacks three things:

- **No product knowledge.** `docs/userguide/` never reaches Studio, so it can't explain triggers, Work, approvals, Requests, runtimes or sign-in beyond their file formats.
- **No recipes.** It knows the skill.md keys but not good patterns, such as when to use a workflow rather than one agent, approval policies, triggers, or CLI runtimes and their `runtime_config.model`.
- **Chat can't start work itself.** Its only way to act is `delegate_to_agent`. Running a workflow or starting a Request happens only on Ask's route (`services/ask/planner.rs` → `startRequest`), which the chat model doesn't know about.

## Wave A — Studio guides (on-demand knowledge)

- [x] **Guide files:** one Markdown file per topic, short and task-shaped, compiled into the binary with `include_str!` (not files in the user's repo; they describe the app, not the user's definitions). Proposed home: `src-tauri/resources/studio-guides/`. Each file has frontmatter `id`, `title` and `when` (one line on when to read it).
- [x] **First topics:**
  - [x] `build-an-agent`: choosing tools, `requires_approval`, `instructions.md` vs skill.md body, model choice
  - [x] `agent-or-workflow`: when a single agent is enough and when steps, approvals or several agents call for a workflow
  - [x] `write-a-workflow`: step shapes, inputs and outputs, approvals, scope
  - [x] `set-up-a-trigger`: schedules and events, what a trigger starts
  - [x] `approvals-and-policies`: NANDI.md, `.nandi/policies/*.md`, per-tool approval
  - [x] `runtimes-and-sign-in`: built-in vs CLI runtimes (container only, company sign-in first, keys last), `runtime_config`
  - [x] `requests-and-work`: what a Request is, Work view, pause, resume, cancel, rating
  - [x] `network-and-delegation`: asking members, `delegate_to_agent`, capabilities
- [x] **One source with the user guide:** the guides and `docs/userguide/userguide.md` must not drift. Pick one, and record the decision here. **Decided (2026-10-20):** the guides are the source. The user guide marks the section that covers each one with `<!-- studio-guide: <id> -->`, and `the_user_guide_covers_every_guide` fails when one is missing. The options were:
  - the guides are the source and the user guide links or quotes them, or
  - a test asserts each guide's `id` has a matching userguide section.
- [x] **Tools** in `services/studio/tools.rs`:
  - [x] `nandi_list_guides` returns `{ id, title, when }`.
  - [x] `nandi_get_guide(id)` returns the body.
  - [x] Both are read-only and need no approval, like the other `nandi_*` tools.
- [x] **Prompt** (`studio_prompt`): add a "## How to do things" section that lists the guide titles and `when` lines (generated from the files, not hand-written), and says to read the matching guide before proposing an agent, workflow or trigger, or explaining a feature.
- [x] **Facts stay generated:** guides must not restate schema keys, tool IDs or model names. A test fails if a guide names a tool ID that isn't in `builtin_tools()`, a frontmatter key that isn't in `SKILL_FRONTMATTER_KEYS` (`guides_name_only_real_tools_and_keys`), or a model (`guides_name_no_models`). This is the same idea as `the_prompt_matches_the_code`.
- [x] **Ask route:** Question answers get the guide index plus the body of the guide the question is most about (`context::describe_guides`, added in `ask::answer`). It isn't a `GatewayContext` field, because the planner doesn't need it.
- [x] **Docs:** add a Studio guides section to `docs/developer/interaction-layer.md`, covering how to add a guide and the drift test.
- [ ] **Acceptance** (needs a live model in the app; the prompt-size item is covered by `the_prompt_lists_every_guide_and_grows_only_by_the_index`):
  - [ ] "How do triggers work?" in Studio reads `set-up-a-trigger` (shown under "Looked at") and answers without inventing keys.
  - [ ] "Build me an agent that summarises my inbox daily" reads `build-an-agent` and `set-up-a-trigger`, then proposes a skill.md and a trigger that both pass `nandi_check_artifact`.
  - [x] The prompt grows only by the index (measure before and after; keep it under ~1 KB).

## Wave B — Chat can start work (one route for Chat and Ask)

- [x] **`nandi_start_request` tool** (Studio only): input `{ text, plan? }`. Built as two calls: without `plan` it returns the plan and changes nothing; with it, `studio::start::gate` checks the plan before approval so the card shows what will really run.
  - Without `plan`, it runs `ask::planner::plan` over a fresh `GatewayContext`.
  - With the plan, it calls `services::requests::local::start_request`.
  - It returns the Request ID, which the reply links as `nandi://request/<id>` (`REQUEST_LINK_PREFIX`).
- [x] **Approval:** gated by `studio::start::gate` (called from the orchestrator for both engines), not by `STUDIO_GATE`, whose `requires_approval_for` covers only `delegate_to_agent`. A plan that can't be read asks for approval. The approval card shows the plan's steps, targets, durations, costs and warnings. Plans that need no approval (`planNeedsApproval` is false: only person and manual steps, which wait and send nothing) still don't ask.
- ~~**`nandi_run_workflow(id, inputs)`**~~ Folded into `nandi_start_request` (2026-10-20): running a workflow is a one-step `workflow` plan, and the tool description says so.
- [x] **Editor context in Chat:** when a Studio chat has an artifact open, the prompt gets its ref (`ArtifactRef`) and Studio can read it with `nandi_get_agent` / `nandi_get_workflow`. Today only Ask's `context::load` sees the open file. StudioView reports the open file (`set_studio_open_artifact`). Note: with a file open, Studio's input still goes to Ask's edit mode (`fixedMode`), so this matters for chats sent while a file is open from elsewhere, and for any later change to that mode.
- [x] **Prompt:** the "Answer from facts" section says to use `nandi_start_request` for "run X", "do Y for me" and "every Monday…" (after reading the guide), and `delegate_to_agent` only for asking one person or agent directly.
- [x] **Tests** (`services/studio/tests.rs`, orchestrator state-machine tests):
  - [x] Chat that starts a Request waits for approval, and declining starts nothing.
  - [x] An approved Request appears on Home as a `request` work item.
  - [x] Unknown targets in a model-supplied plan are dropped, as in the planner.
- [ ] **Acceptance** (needs a live model in the app; the canned-engine version is `chat_starts_work` in `services/studio/tests.rs`): "Run the weekly report workflow" in Studio chat shows one approval card, then a live Request card, and the Request finishes.

## Out of scope

- Writing guides into the user's repo, or letting users author guides (later, maybe as `.nandi/guides/`).
- Retrieval or embeddings. The guide set is small enough to list in full and fetch by ID.
- Memory or compaction for `@studio` (separate decision).
