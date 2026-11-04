# Task: Installed agents stay up to date while keeping your edits

**From:** owner question 2026-10-24: "how will these agents, which may be stale on a user's machine while a new version is pushed, get updated?"
**Size:** M, in two parts. Part 1 first.

## Today

- Updates are manual: Library › Installed › **Check for updates**, then **Update** or **Update all**. Nothing checks on its own (`services/marketplace/mod.rs` `check_for_updates`, called only from the command and HTTP route).
- An installed agent is read-only. Editing any of its files, including the `instructions.md` that Chief of Staff and PR Reviewer ask you to edit, marks it **Modified**, and a modified install is never updated. Forking makes your own copy, which gets no updates at all.

So the agents built to be personalized (your role, your review rules) are exactly the ones that stop getting updates.

## Part 1: Your edits live outside the installed copy (shipped 2026-10-25)

Done: `.nandi/overrides/<agent-id>/instructions.md`, edited in the agent's Settings › Your rules or by Ask (save kind `instructions`), with Reset to default. Catalog: AG-27. Not done: the "default rules changed" diff after an update.

- For a marketplace agent, `instructions.md` becomes **your** file, beside the install, e.g. `.nandi/overrides/<agent-id>/instructions.md`. On first edit it starts as a copy of the shipped one.
- The agent loads `skill.md` from the install and `instructions.md` from your override when there is one (`services/repos/agent_locator.rs`, `domain/agent.rs` `from_files`).
- `tree_hash` covers only the shipped files, so an override never marks the install Modified. **Update** replaces `skill.md` and leaves your rules alone.
- When the shipped `instructions.md` changes in an update, show "The default rules changed" with a diff, so you can merge what you want.
- **Test:**
  1. Edit Chief of Staff's role and PR Reviewer's rules.
  2. Update the pack: both keep your edits and get the new `skill.md`.
  3. Uninstall, then reinstall: your override is still there and offered back.

## Part 2: Updates find you

- Check for updates at startup and every 6 hours, as a background job (`services/marketplace`). Emit `marketplace://update_available`, which already exists.
- Show "Updates available (N)" in Library and an Inbox item. Show the version on each agent's header.
- An **Auto-update official packs** setting (on by default for signed first-party packs): unmodified installs update on their own, and the Inbox says what changed (release notes from the pack's README or a `CHANGELOG.md`).
- **Test:**
  1. Publish a new pack version: within the check interval the Inbox says it's available.
  2. With auto-update on, it is applied, and the agent header shows the new version.
  3. With it off, nothing changes until you click Update.
- **Don't apply an update the registry hasn't signed yet.** Updates are found from the source's git tags, but the registry index is read through GitHub's raw CDN, which caches for up to about 5 minutes. On 2026-10-25, dev-pack 0.4.1 was applied in that window: it installed with `signatureVerified: false` even though the release is signed. Offer an update only once the registry lists the version (or say "waiting for the registry"), or verify the install again later and clear the flag. Until then, after a release, wait about 5 minutes before running Check for updates.
