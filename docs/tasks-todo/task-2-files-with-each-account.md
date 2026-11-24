# Task: Drive and OneDrive (and Calendar) With Each Account

**From:** owner, 2026-11-22: "add ability to read google drive and one drives as well with each connection so you have users drive data access to work on things for 0.3."
**Serves:** state (one sign-in per account, its own cursors) and trust (read-only, asked once, per account). For 0.3.0.
**Builds on:** many mailboxes (`task-2-many-mailboxes.md`, slices 1 and 2 on main): accounts are `MailboxId { provider, email }`.

## Today

- `services/workspace/` reads Outlook Calendar, Google Calendar, OneDrive and Google Drive, read-only, as their own connectors in Settings › Calendar & files.
- Each is a separate sign-in (`workspace/accounts.rs` `AccountKind`, keyring `nandi.<id>.oauth`), one account per kind, so a second Google account can't add its Drive, and connecting Gmail never offers Drive.

## What to Do

### 1. One sign-in per account, asking for what the person ticks

- [x] Connecting a mailbox shows two ticks, on by default: **Also read my files** (Drive or OneDrive) and **Also read my calendar**. The sign-in asks only for the ticked read-only scopes (incremental consent; `account_oauth` already takes the scope list).
- [x] A connected account can add or drop files or calendar later from its row in Settings › Email (a new sign-in for the added scope only).
- [x] Files and calendar for an account use that account's sign-in and keys (`MailboxId`-style names), so each of several accounts has its own Drive cursor and calendar cursor.
- [x] **Upgrade in place:** today's separate calendar and files connections move under the matching account (same address) on first start, with an upgrade test. Nobody signs in again.

### 2. Tools and the record

- [x] `files_*` and `calendar_*` tools reach every granted account and say which account each result came from (like `mailbox:` for mail).
- [x] Grants name the account and what it covers (mail, files, calendar).
- [x] Notes and to-dos that come from a file link back to it and name the account. (Prompt-level: tool results carry `account`, and the `meeting-prep` playbook says to link each file and name its account. Nandi's `agent.md` is at its 900-word limit, so it is unchanged.)

### 3. Docs

- [x] User guide (Settings › Email, the old Calendar & files page folded in or pointing there), `docs/testing/`, `docs/developer/email.md`.

## Rules

- Read-only, always. No copies: a file's text lives for one tool call.
- Tokens stay in the keychain and tools run on the host, never in a container.
- Google's Drive read scope is restricted, like Gmail's: covered by the same Google review.
