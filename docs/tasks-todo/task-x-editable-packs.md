# Task: Installed packs are editable in your own repo, and updates merge with your edits

**From:** owner idea 2026-10-27: "add everything to the user's connected git repo, so the packs they added are checked into their own repo (company or private) and everyone can improve agents without losing code", in place of forcing a fork.
**Size:** M. Post-demo (demo scope rule: noted here, not built now).
**Related:** `task-x-agent-updates.md` (part 1 overrides, part 2 update checks), `task-x-plugin-packs.md`, `task-x-company-governance.md` (admin policies), `task-x-architecture-review.md`. Licensing: `licensing-agreements-only` (memory).

## Today

- **Installs already live in the user's repo.** The installer writes packs into the gold copy at `.nandi/agents/marketplace/packs/<pack>/…` (`services/repos/layout.rs:17-20`) and records each one in `.nandi/installed.yaml` with source, version, commit and `tree_hash` (`marketplace/install.rs:436, 514`). The default `.gitignore` (`repo_bootstrap.rs:20`) leaves them in, so they are committed and pushed to whatever remote the user connected. Everyone sharing that repo already gets the same packs.
- **But vendored folders are read-only** (`layout.rs:168 is_vendored`, `services/agents.rs:540`). Editing one marks the install **Modified** (`install.rs:558`), and a Modified install is never updated.
- **Workarounds today:**
  - **Fork** (`marketplace/mod.rs:799`, `domain/agent.rs:600`) copies the agent under a new ID with no updates at all.
  - **`.nandi/overrides/<agent>/instructions.md`** (agent-updates part 1) handles that one file only.

So a company that improves a shared agent either loses updates or keeps its improvements outside the agent.

## Goal

A pack installed into a company's (or a person's private) repo is an ordinary, editable part of that repo. Improvements are committed and shared through normal sync. Publisher updates merge in without losing them. Giving a change back to the publisher is optional.

## 1. Editable vendored packs

- [ ] Lift the read-only rule for agents and workflows under `.nandi/agents/marketplace/`. Writes still go through the agent and workflow repositories (gold-copy rule), not straight to disk.
- [ ] Keep the install allowlist and validation for what an edit may add, so the same file kinds and the same domain checks apply as on install.
- [ ] **"Modified" becomes a state, not a dead end.** It still means "differs from the installed release" (`tree_hash`), but it no longer blocks updates.
- [ ] Remove the `instructions.md` override special case. Overrides become ordinary edits, and existing override files fold into the pack folder. Nothing is released, so no migration path is needed (no-legacy rule).
- [ ] **Fork stays only as "Make it my own agent"** (new ID, no publisher link). Rename it in the UI and drop it from the update-blocking flow.

## 2. Updates merge with local edits

- [ ] `installed.yaml` already records the base commit. On update, fetch the base and the new release (the existing scratch-repo fetch in `marketplace/source.rs`) and do a three-way merge per file: base → new release, base → local. The model is a git vendor branch. Use git2 merge on trees, not a checkout, so the "no hooks, no symlinks" guarantee holds.
- [ ] **Clean merge:** apply it, commit to the user's repo with the message "Update <pack> <old> → <new> (kept your edits)", and update `installed.yaml` (version, commit, `tree_hash` of the release, plus a `modified: true` flag).
- [ ] **Conflicts:** nothing is written. The update shows as "Needs a decision", with a small screen per file: **Keep mine / Take theirs / View diff** (later, line-level editing). Applying commits only once all files are resolved.
- [ ] **Auto-update** (agent-updates part 2) applies only clean merges and sends conflicts to the Inbox.
- [ ] **Shared repos:** when several people share one company repo, the update merge happens on one machine and reaches the others as a normal commit. Use the repo lock so two machines don't merge the same update at once; the second one sees it is already applied after it syncs.

## 3. Trust, policy and licensing

- [ ] **Show provenance honestly.** Use "Signed by <publisher> 0.4.4" when unmodified, and "Signed by <publisher> 0.4.4, modified by <company/person>" with a diff against the release when edited. The signature check still verifies the release itself (base commit); local edits are labelled, never presented as signed.
- [ ] **Admin policy** (company governance): "Only unmodified signed packs" and "Who may edit installed packs". It is enforced in the same evaluator and shown as "Managed by your organization".
- [ ] **Licensing:** the pack license or agreement states whether modification inside the licensee's repo is allowed. The official pack's agreement should allow it for licensees, in line with "agreements only". Show the license on the pack page before the first edit.

## 4. Optional: give a change back to the publisher

- [ ] Add a **Suggest this change** action on a modified agent. It builds a branch from the base commit plus the local diff and opens a PR to the pack's repo. It pushes straight to the pack repo when the user has write access (company-owned packs); otherwise it uses a fork created behind the scenes (the user never manages one). Reuse `marketplace/github.rs` (`fork`, `create_ref`, `put_file`, `create_pr`) and the flow in `marketplace/publish.rs`.
- [ ] Only the agent's files are sent: no run content and no other repo files. Show the diff before sending.
- [ ] Once merged and released, the change comes back as a normal update and the three-way merge sees no conflict.

## Test

1. Install the official pack into a company repo and edit Chief of Staff's `skill.md` and a workflow. Both save, commit, and appear on a second machine sharing that repo.
2. Release a new pack version that changes different lines. The update merges cleanly, both the edits and the new lines are present, and the agent header shows "modified by <company>".
3. Release a version that changes the same lines. The update shows "Needs a decision"; Keep mine and Take theirs each give the expected file, and nothing is written before Apply.
4. With the "Only unmodified signed packs" policy on, editing is refused and the reason is shown in plain words.
5. Suggest this change: a PR opens on the pack repo with only that agent's diff. After it is merged and released, the update applies with no conflict.
6. Make it my own agent: a new ID, no publisher link, no updates.

Update the feature catalog in `docs/testing/` (marketplace and agent-update entries) in the same change.
