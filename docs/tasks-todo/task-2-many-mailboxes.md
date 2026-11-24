# Task: Many Gmail and Outlook Accounts

**From:** owner, 2026-11-22: "add ability to add multiple gmail and outlook accounts as lot of people have many accounts ... make a campaign for it in next release".
**Serves:** state (each mailbox keeps its own sign-in, cursor and notes) and trust (a grant names the mailbox it covers)
**Order:** the next app release after 0.2.2. The campaign waits for it: `nandi-cloud/docs/campaigns/posts/2026-12-21-linkedin-every-inbox.md`.

## Today

`EmailSettings.accounts` is already a list, but everything is keyed by provider, so it holds one Gmail and one Outlook at most:

- Connecting drops any account of the same provider (`services/email/mod.rs`, `accounts.retain(|a| a.provider != provider)`).
- `EmailSettings::account(provider)` (`settings.rs`) finds the one account.
- The sign-in's keyring entry and the cursor, state and backfill keys are `email.{provider}.*` (`provider.rs`).

## What to Do

Each slice lands on its own.

### 1. One key per mailbox (state)

- [x] An account is found by provider **and** email. Keyring entries and the cursor, state and backfill keys include the mailbox (for example `email.gmail.<hash of the address>.cursor`).
- [x] **Upgrade in place:** existing keys move to the new names on first start, with an upgrade test. Nobody connects again, and no mail is read twice.
- [x] Connecting a second Gmail adds it. Connecting the same address again refreshes its sign-in, as now.
- [x] Sign out, Pause, Resume and the background read act on one mailbox.

### 2. Settings › Email shows them all (UI)

- [x] A list of mailboxes, each with its provider, address, status and its own Pause and Sign out. **Add another account** for Gmail and for Outlook.
- [x] The **first mailbox stays this Nandi's identity** (relay, trial link, Settings › Account), marked as such. Adding more never changes it.

### 3. Agents know which mailbox (trust)

- [x] Mail tools take an optional mailbox; without one they search all granted mailboxes. Results say which mailbox each message came from.
- [x] Grants name the mailbox. Connecting is still the person's say-so for Nandi to read that one mailbox, read-only. A grant covers every mailbox unless the person narrows it (the access request, Settings › Connectors › Access); grants saved before cover every mailbox, as they did.
- [x] Notes and to-dos keep their source mailbox, so "from your work inbox" is on the record. Notes cite it through Nandi's `agent.md`; the to-do record has no mailbox field (it would need a migration), so `todo_add` asks the model to name the mailbox in the text.

### 4. Docs and the record

- [ ] The user guide (Settings › Email), `docs/testing/` entries, the changelog. Then mark the campaign post Ready.

## Not in This Task

- Sending mail, or any write. Mail stays read-only.
- A unified inbox UI. Nandi's notes and to-dos are the unified view; the model does the reading.
