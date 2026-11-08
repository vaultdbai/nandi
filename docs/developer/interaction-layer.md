# Wave CD: Interaction Layer

One ask box everywhere, Studio as a split editor, a three-state
companion, and a Work view for long-running workflows.

## The ask box

`AskBox` (`src/components/ask/`) is the same component in every place an
input appears: docked under the main pane, in Studio's chat pane, in the
companion, and at the bottom of a workflow run's project view. Only its
context and placeholder change.

```text
AskBox → useAsk → client.askStreaming(AskRequest) → services/ask
                                                    ├─ context::load      (once per ask)
                                                    ├─ classifier::classify
                                                    └─ answer | planner::plan | editor::preview
         ← ask://streaming_delta … ask://completed  (then ask_history)
```

### Context

`AskContext { currentView, focusedArtifact, recentRuns }` comes from the
view (`askContextForView` in `src/lib/ask.ts`):

| View                                 | `currentView`                      | Focused artifact           |
| ------------------------------------ | ---------------------------------- | -------------------------- |
| Home (dashboard)                     | `home`                             | none                       |
| Agent                                | `agent:<id>`                       | `{ agent, id }`            |
| Capability                           | `capability:<id>`                  | `{ capability, id }`       |
| Workflow, workflow builder, run view | `workflow:<id>`                    | `{ workflow, id }`         |
| Member profile                       | `member:<instance>`                | none                       |
| Gateway                              | `gateway`                          | `{ gateway }`              |
| Run views                            | the view kind, run in `recentRuns` | none                       |
| Studio, Work, companion              | `studio`, `work`, `companion`      | the open artifact (Studio) |

`services/ask/context.rs` turns it into a `GatewayContext`: active runs,
completed outputs from the last week (runs the user was looking at
first), capabilities with how this instance relates to them (owned,
provided, used), agents, workflows, policies (NANDI.md's block and
`.nandi/policies/*.md`), network members, and the focused artifact's file.
It's loaded once per ask and shared by every step.

### Classify → route → respond

- **Classifier** (`classifier.rs`): the gateway's model returns
  `{ kind, confidence, reason }` from a scoped prompt (view and focus
  only). If the model is unavailable or unreadable, a keyword heuristic
  decides. Below `MIN_CONFIDENCE` (0.5) the box asks back
  (`AskResponse::Clarification`).
- **Question**: answered from the context in Markdown, streamed; the last
  line `FOLLOWUPS: a | b | c` becomes follow-up buttons, and the IDs the
  answer mentions become source links.
- **Action** (`planner.rs`): the model may only name listed capabilities,
  agents, workflows, and members; unknown targets are dropped and a
  keyword matcher stands in when it can't answer. One capability → a
  delegation; several → one step each in the order asked (a workflow);
  an existing workflow → run it; a local agent → message it; nothing →
  `rejection` with the reason. Durations, costs, and warnings come from
  the context (SLAs, per-call costs, providers online, approval
  policies). **Run plan** saves the plan as a **Request**
  (`startRequest`, `services/requests/local.rs`, task 14) and opens its
  page. The core carries it out: steps start in order, each linked to
  the run, workflow run or task it started (`request_steps`), and a
  runner per Request follows them, emits `request://updated`, and picks
  up again after a restart (`resume_all`). Plans that only point at
  people or manual steps start without asking (`planNeedsApproval`).
  Person and manual steps wait for **Mark done**
  (`completeRequestStep`). **Do this again** (`rerunRequest`) starts the
  same plan as a new Request; **Save as workflow**
  (`draftWorkflowFromRequest`) drafts a workflow file the user confirms,
  and `createWorkflow` validates it. Local Requests list on Home as work
  items of kind `request`; the runs, workflow runs and tasks they start
  don't list on their own.
- **Edit** (`editor.rs`): the artifact is the focused one, or the one the
  ask names. The model rewrites only that file; the result must parse as
  that artifact. Capability edits get the smallest legal version bump
  (`studio/version_bumper.rs`) when the model left the version alone,
  and are refused when it bumped too little, went down, or changed the
  ID. The preview carries a unified diff (libgit2), and its impact names
  subscribers (local providers, network members offering it), workflows
  using an agent, and runs in flight. **Apply** opens it in Studio.

Every model call goes through `llm::stream`, which uses the default model
when its provider has a key and otherwise a provider the user has set a
key for (`llm_keys::usable_model`).

### Chat and Ask

In a conversation (an agent's page, a Studio chat) the box
offers **Chat** and **Ask**. Chat sends a plain message the way the input
always did (agent pages start in Chat); Ask goes through the ask service.
The companion has one input with no toggle (see `companion-window.md`).

### Streaming and history

`ask_streaming` returns a request ID at once; `ask://streaming_delta`
events carry the model's text, then `ask://completed` carries the
response (or the error). `useAsk` buffers events for request IDs it
doesn't know yet, so a fast answer is never lost. Finished asks are kept
in `ask_history` (v14) and listed by `list_ask_history` (`AskHistory` in
Studio and the companion).

## Studio

A split view: the gateway chat (conversations, the ask box, recent asks)
on the left; the artifact editor on the right with Capability, Agent,
Workflow, Policy (including NANDI.md), and Tool tabs. Tools are a
read-only catalog.

- `load_artifact` returns the raw file and `{ frontmatter, body }`; a
  capability, policy, or NANDI.md that doesn't exist yet loads as a
  starter template.
- Forms edit the raw file line by line (`src/lib/yaml-fields.ts`), so
  formatting and comments survive.
- `save_artifact` validates the text as that artifact and writes it
  through its owner (capability service, agent and workflow repositories,
  `PolicyRepository`, the gateway's `save_manifest`), so projections and
  events follow as for any edit. Then `GitService::commit_files` commits
  just that file on top of `HEAD`.
- Push targets: `localOnly` (commit), `remoteBranch` (commit and push to
  `<prefix>/<slug>` through the remote service, which never pushes to
  main or master), `remoteBranchAndPr` (push; opening the pull request
  isn't implemented yet and comes back as a warning).
- Capability saves must bump the version as much as the change needs:
  schemas, required tools, or visibility → major; other policy, routing,
  optional tools → minor; text → patch.

### Studio guides

Studio knows how to do things in Nandi from short guides in
`src-tauri/resources/studio-guides/*.md`, compiled in with `include_str!`
(`services/studio/guides.rs`). They describe the app, not the user's
definitions, so they never live in the user's repo.

- Each file has frontmatter `id`, `title`, and `when` (one plain line),
  then a task-shaped body: steps and judgement, not reference.
- Studio's prompt has a "How to do things" index generated from the files
  (`guides::index_lines`, kept under 1 KB). Studio fetches a body with
  `nandi_get_guide` (`nandi_list_guides` lists them). Both are read-only.
- Ask's question answers get the index plus the body of the guide the
  question is most about (`context::describe_guides`).
- **Facts stay generated.** Guides don't restate schema keys, tool IDs, or
  model names; the prompt does. `guides_name_only_real_tools_and_keys`
  fails when a guide names a backticked snake_case identifier that isn't a
  built-in or Studio tool, `delegate_to_agent`, or a skill.md key.
- **One source with the user guide.** The guides are the source. The user
  guide marks the section that covers each one with
  `<!-- studio-guide: <id> -->`, and `the_user_guide_covers_every_guide`
  fails when one is missing.

To add a guide: write the file, add it to `SOURCES` in `guides.rs`, add
its anchor to `docs/userguide/userguide.md`, and run
`cargo test --lib studio::`.

### Chat starts work

`nandi_start_request` (`services/studio/start.rs`) puts Studio chat on
Ask's route. The first call, with only `text`, plans over a fresh
`GatewayContext` (`ask::planner::plan`) and returns the plan without
changing anything. The second call, with the plan, starts the Request
(`requests::local::start_request`) and returns `nandi://request/<id>`.

Before the second call runs, `start::gate` (called from the orchestrator
for Studio's turns) swaps the model's plan for a checked one: unknown
targets are dropped and estimates come from the context. The call waits
for approval when a step acts (an agent, a workflow, a delegation), the
same rule as Ask's `planNeedsApproval`. The approval card shows the plan
(`ActionPlanPreview`). Running an existing workflow is a one-step plan, so
there is no separate run-workflow tool.

### The Chief of Staff acts on what arrives

New mail, a calendar change or a chat can call for work: a bill due, a
birthday, a colleague's ask. The Chief of Staff sets it going with three
host tools (`services/requests/tool.rs`, offered only to it, in chats and in
the email and calendar checks' runs):

| Tool          | Does                                                                                                                    |
| ------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `work_open`   | Lists open local Requests and the intake queue, so news goes to work already going and nothing is queued twice          |
| `work_start`  | Plans `what` over a fresh `GatewayContext` and starts the Request (`tool::start_now`)                                   |
| `work_update` | Adds news (`local::add_update`): saved in `request_updates`, appended to the input, sent to the agent working on it now |

**What comes from outside waits for the user** (`services/intake/`). From
the email or calendar check (and any outside source added later:
`tool::from_outside`), `work_start` and `work_update` don't act. The item
is screened, saved in `intake_queue` and waits for the user's approval
(`approve_queued_work`, `dismiss_queued_work`,
`POST /api/chief-of-staff/queue/{id}/approve|dismiss`). Approving starts
the work or adds the news. In the user's own chat the tools act at once:
the user is there.

**Teammates' requests take the same intake** (owner: "same intake for
everyone"). `Executor::intake` runs for every task from another Nandi,
right after `TaskDelivered`, through `intake::gate_task`. It first asks
`admit` (`Executor::standing`): a task `admit` refuses for any reason other
than "not trusted yet" (`deny`, paused, blocked, no such agent or
capability) is left to `admit`, which refuses it as before.

| Screening  | Admit says                  | Result                                                                                                               |
| ---------- | --------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Blocked    | any                         | Refused (`NotAccepting`) with the reasons                                                                            |
| Clean      | admitted (trusted)          | Runs at once, as before                                                                                              |
| Clean      | not trusted yet (`ask`)     | Queued (`action: run`); the task waits, still `Delivered`, "Waiting for the owner to approve" (`NeedsTheirApproval`) |
| Suspicious | admitted or not trusted yet | Queued, with the warnings                                                                                            |

Tasks for the gateway are only screened (`screen_only`): the gateway
already holds requests for the owner in the Inbox. Approving lets that one
task past the `ask` trust check (`intake::is_approved_task` in `admit` and
`admit_workflow`); dismissing refuses it as `Declined`. A requester's
cancel or the task's deadline takes it off the queue (`drop_task`). The
queue item survives a restart: the resumed task finds its decision.

**Auto-approve rules** let some items past the click. The user's rules are
the local setting `intake.autoApprove` (`AutoApproveRule`: an ID, a
`RuleMatch` and the `IntakeAction`s it allows), read and replaced with
`get_auto_approve_rules` / `set_auto_approve_rules`
(`GET`/`PUT /api/chief-of-staff/auto-approve`). The core validates the whole
list on save (`services/intake/rules.rs`): at least one action per rule, a
valid address, a bare domain like `chase.com`, a teammate picked, at most 100. Rules run after screening, never instead of it: only a clean item
can match; suspicious items still queue and blocked ones are still refused.
An `emailAddress` or `emailDomain` rule matches only mail whose sender
passed DMARC (`dmarc_verified`, from `Authentication-Results` in
`services/email/mail.rs`), so a forged `From:` never matches; a domain also
covers its subdomains. A `teammate` rule matches the signed instance ID and
applies to `run`. An item a rule approves is noted in its thread
("Approved automatically (mail from chase.com)") and shows in "Since you
were away". The UI is Settings › Auto-approve (`AutoApproveSection.tsx`,
logic in `auto-approve-model.ts`), plus "Always allow mail from <domain>"
on a clean queued email item (`QueueRow.alwaysAllowDomain`,
`useAlwaysAllow`). A teammate's queued item has no such shortcut: the
queue doesn't carry their instance ID.

**Each item has a Chief of Staff thread** (`intake/thread.rs`). A
conversation `intake:<item id>` owned by the Chief of Staff, so it runs
with its skill, notes, grants and permissions, is made when the item is
queued. News for the same work joins that thread (`thread::for_request`).
What happens is noted in it as the Chief of Staff's messages (queued,
with the screening and the plan; approved; dismissed). Work to start is
planned when it's queued (`tool::plan_for`), so the user approves who does
it, and Approve starts that plan.

The user chats in the thread (`send_intake_message`,
`POST /api/chief-of-staff/threads/{id}/messages`; the page loads
`get_intake_thread` and the conversation): draft a reply, hand it to an
agent, add what's missing. The orchestrator gives those turns the task
(`thread::prompt`, inside `<task-data>`, untrusted). Work the Chief of
Staff starts or news it adds there always queues for approval in the same
thread (`requests::tool::call` with the thread), even when the user asked
for it: Approve is a click, never a chat message. Home's rows open the
thread, and a step waiting on the user can be marked done from Home
(`DigestItem.waiting_step`, `complete_request_step`).

**Screening** (`intake/screen.rs`, pure, offline) reads the request, and
the email itself when the call names its `message_id`. No link is opened
and no attachment is read (Nandi never downloads attachments; a malware
scanner plugs in here later).

| Sign                                                                                                                    | Result  |
| ----------------------------------------------------------------------------------------------------------------------- | ------- |
| A link to a bare IP, a punycode host, an `@` in the link's host, or a program or script download                        | Block   |
| A sign-in or account lure with a link; a sender domain that imitates the user's (one or two letters, or another ending) | Block   |
| A shortened or `http://` link; a payment lure; urgency; a sender borrowing a brand or showing another address           | Warning |

Any block, or three warnings, blocks the item: it can only be dismissed.
Warnings show with the item before the user approves it. Free-mail
domains are never lookalikes.

- **Never back to itself.** `tool::for_the_user` drops plan steps that
  target the Chief of Staff. A plan with nothing left becomes one manual
  step, "For you: …", so the work still shows as the user's to do.
- **Who started it** is the Request's `source`: `email`, `calendar` or
  `chiefOfStaff` (`tool::source_of` reads the run); the ask box and Studio
  stay `me`.
- **Since you were away** (`requests/digest.rs`, `get_chief_of_staff_digest`,
  `GET /api/chief-of-staff/digest`): the queue waiting for approval, then
  Requests the Chief of Staff started, news it added and work that finished
  since the user last looked (`mark_chief_of_staff_digest_seen`). Work
  waiting on the user stays until it's done. Home shows it under the email
  to-do.
- The email batch tells it not to start work from backfill, and on the
  first read only for what is still open and due soon. Its behaviour is
  skill.md › Acting on what arrives.

### Chat saves what it builds

`nandi_save_artifact` (`services/studio/save.rs`) writes a definition the
chat built: `kind` agent, workflow, capability or policy (`content`, saved
through `artifact_saver` like the editor, which now creates new agents and
workflows too), trigger (`trigger`, a `TriggerInput` for the trigger
service) or connector (`connector`, through `connectors::declare_connector`:
a built-in by ID, or `kind: mcp` with config and `secret_refs`). Agents and
workflows are checked as `nandi_check_artifact` checks them first; a failed
check returns `saved: false` with the errors and writes nothing.

`save::gate` (orchestrator, Studio turns) makes every call wait for
approval; the card reads "Save the agent inbox". Secrets never pass
through: trigger secrets are generated and not returned, and connector
config refuses credential-like keys. `nandi_list_connectors` shows the
catalog and what's installed.

The file open in Studio's editor reaches the chat prompt too
(`set_studio_open_artifact` → `PromptFacts::open_artifact`): "this" means
that file, and Studio reads it with `nandi_get_agent` / `nandi_get_workflow`.

## Companion

Three states in `CompanionWindow`:

| From      | To        | How                                  |
| --------- | --------- | ------------------------------------ |
| collapsed | expanded  | click the bison, or the hotkey       |
| expanded  | overlay   | Expand, or Mod+Shift+F               |
| overlay   | expanded  | Collapse, Mod+Shift+F, or Escape     |
| expanded  | collapsed | Escape, a click outside, or Minimize |

Collapsed, the window is the bison (`CompanionCollapsed`, drawn by `BisonPose`: each pose image stacked as its body plus copies cut to the head, tail, crates and badge, which turn about their joints on loops of different lengths; the head's edge is a feathered mask so it never shows a seam; shapes and pivots are generated from the 384 px poses into `bison-pose.css`): its pose
(`poseFor`: `wait`, `carry`, `rest`, `done` for a moment after the Inbox
shrinks) and ring colour say what matters most, with a count badge. With
**Nandi on the desktop** (`companion.bubble`, on by default) it's shown at
start and always on top, and closing the card goes back to it instead of
hiding the window. The window keeps to a screen corner: Rust's
`place_companion` sizes it for the bison or the card and puts it in the
saved corner (`companion.corner`), and a drag snaps to the nearest corner
when the moves stop (`commands/companion.rs`, pure `nearest_corner` /
`corner_position`). The window-state plugin denylists it. The card opens
on `companion://expand` (the hotkey, Watch in companion) or a click, not
on focus, so pressing the bison to drag it doesn't open it; a press that
moves more than 5 px starts the OS drag instead of a click. The card and overlay show the
ask box, Needs you (approvals and failures, with Answer), Running, and
Recent (`useCompanionFeed`). Standalone, the window resizes itself (the
overlay spans the monitor); in the browser the companion docks in the
main window's corner. Links open in the main window over
`companion://navigate`, except a Request: the first open pins it in the
companion as a live card (watch it, ask about it in the chat;
`requestToPin`), and only the pinned card's › opens its page in the main
window.

## Work

`services/work` lists workflow runs and the tasks this instance
delegated as `WorkItem`s: Active, Waiting on me (approvals, paused runs,
questions), and Done in the last 24 hours. Each has progress and its next
action. Finished delegations can be rated 1–5 (`rate_work`, stored on
the requester's capability evidence).

A workflow run opens as a project (`WorkflowProjectView`): owner, start,
an ETA from the average step so far, progress, Pause / Resume / Cancel,
the workflow graph, a timeline, what's coming up, and an ask box scoped
to the run.

Workflow runs can be `paused` (the driver stops before the next step;
resume picks up where it stopped) and `cancelled` (the step's agent turn
is stopped too). `update_run` never lets a driver's save overwrite
either.

## Interfaces

| Area   | Commands                                                                              | Routes                                                                                                           |
| ------ | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Ask    | `ask`, `ask_streaming`, `list_ask_history`                                            | `POST /api/ask`, `POST /api/ask/stream`, `GET /api/ask/history`                                                  |
| Studio | `load_artifact`, `save_artifact`, `preview_edit`, `list_push_targets`                 | `POST /api/studio/{load,save,preview}`, `GET /api/studio/push-targets`                                           |
| Work   | `list_work`, `get_work_item`, `pause_work`, `resume_work`, `cancel_work`, `rate_work` | `GET /api/work`, `GET /api/work/{id}`, `POST /api/work/{id}/{pause,resume,cancel}`, `POST /api/work/{id}/rating` |
| Models | `get_default_model`, `set_default_model`                                              | `GET` / `PUT /api/default-model`                                                                                 |

Events: `ask://streaming_delta`, `ask://completed`,
`studio://artifact_loaded`, `studio://artifact_saved`,
`studio://edit_previewed`, `work://item_updated` (also `item_added` and
`item_removed`, reserved).

`ArtifactRef` serializes as `{ kind, id }` (`{ kind: "gateway" }` for
NANDI.md); `AskResponse` as `{ kind, data }`.

Migration v14 adds `ask_history` and `work_preferences` (machine-local;
neither is a projection of the gold copy).
