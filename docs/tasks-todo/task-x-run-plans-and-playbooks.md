# Task: Run plans, playbooks and site accounts

**From:** owner, 2026-11-08. "The Chief of Staff should always prepare a plan for the work it gets, step by step (launch the website, log in, go to my account, pay, confirm), take it to its conclusion including closing the browser, and take screenshots for the logs at important places. It will acquire many capabilities, so give it many skill files; it must plan at a high level, know how to delegate, and build workflows that get work done."
**Status:** in progress, built step by step (owner: "do it one by one overnight").
**Builds on:** Requests (the work plan: who does each step), the intake (queue, threads, auto-approve), the browser pilot (`services/pilots/browser.rs`: persistent per-agent profile, keyring secrets typed by name, screenshots, `browser_close`, `browser_hand_over`), orchestrator middlewares (`services/orchestrator/middleware.rs` plan mode, `verify_on_stop.rs`), Studio guides (`services/studio/guides.rs`: an index in the prompt, bodies fetched on demand).

## Decisions (taken overnight on the owner's behalf, as proposed; change any)

- [x] **Plan first applies to** any run that changes something outside the agent's notes (browser typing, clicking, submitting; starting or updating work; delegating; writing files outside `my-notes/`), for every agent. Pure questions need no plan.
- [x] **Screenshots** stay local in the run's files, are kept 30 days, and never go into backups or the repo.
- [x] **One approval:** a queued item's Approve also approves its run plan; only hand-over steps (sign-in, payment, posting, anything irreversible) ask again.

## Two levels of plan

- **Work plan** (exists): who does what, a Request's steps.
- **Run plan** (new): how one run does its part, step by step. Example, paying a bill:

  | #   | Step                         | Kind     | Needs        | Evidence                 |
  | --- | ---------------------------- | -------- | ------------ | ------------------------ |
  | 1   | Open acme-energy.com         | navigate | site allowed | screenshot               |
  | 2   | Sign in                      | handover | the user     | none (never shoot it)    |
  | 3   | Billing › Pay bill           | navigate |              |                          |
  | 4   | Enter $84 (from the bill)    | fill     |              | screenshot before submit |
  | 5   | Pay                          | handover | the user     |                          |
  | 6   | Read the confirmation number | verify   |              | screenshot               |
  | 7   | Note it, close the browser   | cleanup  |              |                          |

## Step 1: Run plans, enforced

Built 2026-11-08. Developer doc: `docs/developer/agent-runtime.md` › Run plans; catalog AG-32. Decided while building: shell commands aren't classified as changes; plan tools and both middlewares are off in email and calendar check runs, and an agent with no tools gets no plan tools; screenshots auto-link to the `doing` step.

- [x] **Plan tools** for every agent, answered on the host: `plan_set` (the steps: title, kind `navigate | read | fill | submit | handover | approval | delegate | verify | cleanup | other`, `evidence: screenshot | none`, optional note) and `plan_update` (a step's status `pending | doing | done | skipped | failed`, a short result, evidence file names). One plan per run, stored with the run (run data, not the gold copy), replaced by a later `plan_set` (kept as a revision).
- [x] **Live checklist** wherever the run shows (chat, intake thread, Run Detail): a card with each step, its status, its evidence (screenshots as thumbnails linking to the file), updated by events.
- [x] **Plan first** (a middleware, like plan mode): the first call that changes something (see Decisions) is blocked with "Make a plan first (`plan_set`)" when the run has no plan. Reads, notes and the plan tools never are.
- [x] **Risky steps are the user's:** a browser submit, payment, secret or sign-in, `work_start`/`delegate_to_agent`, or a hand-over is allowed only while the current (`doing`) step is of a kind that covers it (`submit`, `handover`, `approval`, `delegate`); else blocked with what to plan. Approvals that already exist still apply on top.
- [x] **Finish check** (like verify-on-stop, once per run): a run that ends with steps not done/skipped, steps marked `screenshot` without evidence, or the agent's browser still open gets one continuation to finish, record evidence, close the browser and report. Its final message summarizes each step.
- [x] **Safety net:** when a run ends, Nandi closes that agent's browser session (pages, not the profile).
- [x] **Screenshots:** `browser_take_screenshot` saved per step into the run's files (`browser/<run>/<step>-<n>.png`), linked in `plan_update`; never on a `handover` step; files older than 30 days removed; excluded from backups.
- [x] Chief of Staff `skill.md`: "always plan" in one or two lines (≤900 words); the playbooks (step 2) carry the detail.
- [x] Tests: plan tools, plan-first blocking, risky-step blocking, finish check firing once, browser closed at run end, screenshot naming and cleanup, the card model.

## Step 2: Playbooks (skill files)

Built 2026-11-08. Developer doc: `docs/developer/agent-runtime.md` › Playbooks; catalog AG-33. Decided while building: frontmatter keys are only `name`, `description`, `when` (others refused, `model:` with its own message); playbooks are offered with the plan tools (not in the email or calendar checks, not to an agent with no tools) and only when the agent has a valid one; broken files are left out of runs and shown on the tab; a built-in agent's playbooks are written once, when it has no `skills/` folder, so edits and deletions stay; the agent's fingerprint includes its playbooks; installs bring `skills/*.md`; Studio can't save playbooks yet (the user edits them on the tab).

- [x] **`skills/` next to an agent's `skill.md`**: Markdown files with frontmatter `name`, `description`, `when`; body ≤900 words each. The domain validates them (unique names, word limit, no `model:`); the repository reads and writes them (gold copy). `domain/playbook.rs`, `AgentRepository::{load_playbooks, save_playbook, delete_playbook}`.
- [x] **Progressive loading:** the prompt carries only the index (name, description, when); `skill_list` and `skill_get(name)` fetch bodies on demand, like Studio's guides. `services/playbooks.rs`, answered on the host in `run_call`; not changes for the run plan.
- [x] **Built-in playbooks for the Chief of Staff**, seeded like its `skill.md` and editable: `web-task`, `pay-a-bill`, `sign-in-and-accounts`, `delegate-work` (do it myself, an agent, a workflow or a person, and how to hand over well), `build-a-workflow`, `email-triage`, `meeting-prep`, `occasions`. 390 to 600 words each; its `skill.md` points at them in one line (898 words).
- [x] Agent page: a Playbooks tab to read and edit them (Markdown editor as for skill.md). List, editor with word count, new, delete; read-only for installed agents; commands `repo_agent_playbooks`, `repo_save_agent_playbook`, `repo_delete_agent_playbook` (and HTTP).
- [x] Tests: validation, index in the prompt, `skill_get`, seeding, editing. Plus: shipped playbooks name only real tools (shares `studio::guides::known_names`), installs keep `skills/`, the tab's model and component.

## Step 3: Sites and accounts

Built 2026-11-08. Developer doc: `docs/developer/pilots.md` › Sites and accounts; catalog AG-34. Decided while building: it lives on the agent's Permissions tab under each browser pilot (where secret values were already set), not in app Settings; a new secret's keychain entry is `nandi.pilot.<agent>.<name>` (lowercase, `_` → `-`), shared by both browsers of the agent; only agents made on this computer (the Chief of Staff too) can add names, pack agents only set values; a secret's name can't be removed from the tab (edit skill.md), only its value; the notes guard applies to every agent writing into `my-notes` through the filesystem connector, not to the user's own edits on My notes; a capitalised secret name (`Password: ACME_PASSWORD`) passes it; Nandi masks secret values in browser replies too, on top of Playwright's own masking.

- [x] **Site secrets in Settings** (per agent, per site): add, edit, remove secrets in the keyring under the names the browser pilot types (`services/pilots/browser.rs` `read_secrets`); the model never sees values. `add_pilot_secret` (skill.md through the repository, then the keychain), `set_pilot_secret`, `clear_browser_profile`, commands and HTTP; `PermissionsPanel` with `site-secrets-model.ts`.
- [x] **Site profiles in notes:** `my-notes/private/sites/<site>.md` for non-secret details a site needs (customer number, billing address, how the site works, which secret name to use). The `sign-in-and-accounts` playbook says how to keep them, and `web-task` points at it. Passwords, card numbers, CVVs and one-time codes are never written anywhere: always handed over. `domain/note_guard.rs` refuses Luhn-valid 13 to 19 digit runs and `password:` style lines in `my-notes` writes.
- [x] **Profile care:** per agent and pilot, "Sign out everywhere" (clear the persistent profile) in the agent's settings; profiles excluded from backups. `browser::sign_out` stops that browser and deletes only its profile, after a confirm; `browser::profile_backup_exclude` in the S3 backup.
- [x] Tests: secret names round-trip, values never in prompts or logs, clear profile.

## Step 4: The Chief of Staff in its own plans, and workflows

- [ ] **A work-plan step the Chief of Staff does itself**, run in the task's intake thread (not its main chat). `requests::tool::for_the_user` stops dropping it; a guard stops a self-step from adding further self-steps without the user's approval.
- [ ] **Save as workflow:** when a plan succeeded and the work recurs (or succeeded twice), the Chief of Staff proposes saving it (`draft_workflow_from_request`), the run plan's hand-overs becoming manual steps; the user approves.

## Step 5: The user's own playbooks

- [ ] The user adds playbooks (Playbooks tab, or "remember how I do this" in chat, which drafts one for approval).
- [ ] The Chief of Staff proposes a playbook once from repeated work, in `suggestions.md`; accepted ones are written to `skills/`.
