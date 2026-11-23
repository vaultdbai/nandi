# Email: Nandi Keeps My Notes from Mail

Nandi reads the user's Outlook or Gmail and keeps My notes up to date. The code is in `src-tauri/src/services/email/`. It has no Tauri imports. The sign-in and token refresh are in `services/auth/oauth.rs` (the shared OAuth engine, also used by the GitHub sign-in).

**Read-only.** Nandi reads mail and writes notes. Nothing is sent, moved, deleted or marked read: the scopes are `Mail.Read` and `gmail.readonly`, and the only HTTP method the mail client has is `GET` (`email/api.rs`).

```text
Settings › Email › Connect ─ PKCE sign-in ─▶ keyring nandi.email.<provider>.<hash>.oauth
                                            + config.json email.accounts + a saved grant (nandi, read)
scheduler (every N minutes) or Check now:
  each connected, unpaused mailbox ─ changes since cursor ─▶ filter ─┐
  all new mail, newest 60 ─▶ ONE Nandi run (source "email") ┘─▶ cursors saved
the run: mail_* tools (host) + filesystem__* notes tools, no container, nothing else
```

## Decisions

| Decision                       | What it means in the code                                                                                                                                 |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Our own connector, not MCP     | `email/tool.rs` runs on the host. Tokens never go into a container                                                                                        |
| Notes, not copies              | Only the change cursor (`local_settings`) and the notes are kept. Headers live for one check or one tool call                                             |
| One run per check              | `check_all` hands every mailbox's new mail over in one `chat::submit`, only when there is some                                                            |
| Email is untrusted             | Mail sits inside `<email-data>`; `text::neutralize` stops it closing the block; email runs get only mail and notes tools                                  |
| Grants per agent and connector | The one grant model ([connector-hub.md](./connector-hub.md)): connecting gives Nandi a saved read grant; disconnecting removes every grant on the mailbox |
| Not in the Library             | Mailboxes are in Settings › Email, not `connectors.yaml`: the connection belongs to this computer's keyring, not the gold copy                            |

## At first-run setup and on Home

- Setup opens with email (step 1, `StepEmail.tsx`, skippable; the order is `ONBOARDING_STEPS` in `identity-model.ts`). One sign-in covers which mailboxes (Outlook, Gmail or both, connected in turn with `ConnectMailbox`) and how often (5, 15, 30, 60 or 120 minutes; **30 is the default**, `DEFAULT_INTERVAL_MINUTES` in Rust and the UI). Skip and Continue both save the interval and go to the name step.
- **Connected:** the name step defaults from the mailbox (`mailboxIdentity`, `draftFromMailbox`) and reads "You're <name> (<email>) · Change": the name and address from the sign-in (Gmail: its profile and userinfo; Outlook: `/me`), the handle from the address's local part through `slugify` and `validateSlug`. Outlook also gives title, department and manager (`email/profile.rs`: Graph `/me?$select=jobTitle,department` and `/me/manager`, both inside `User.Read`), stored on the `EmailAccount` (`profile`) and sent to the UI as `MailboxStatus.profile`. Reading them is best effort: a failed call leaves them empty and never fails the sign-in. Every value stays editable (Change in setup, Settings › Account after, which says when the name came from the mailbox).
- **Skipped:** the name step asks, as before. Home's email to-do stays until a mailbox is connected.
- Not read yet: time zone and working hours need `MailboxSettings.Read`, a new scope that waits for its own change.
- Finishing setup calls `check_email_now`, so the first read starts at once.
- Connecting writes the mailbox owner into Nandi's My role (`builtin_agents::note_identity`, through the agent repository): the template's "Name and title" placeholder becomes "<name>, <email>", a line the user wrote gets the address added, and a role without that line is left alone. Every Nandi run then knows who "me" is. With a profile, the name line also carries the job title ("<name>, <title>, <email>") and `profile::note_profile` fills the "Team and manager" placeholder ("<department>, reports to <manager> (<title>)"). A line the user wrote is never replaced.
- Home's `EmailTodo` shows "Connect your email" while nothing is connected, then a one-line offer of the other provider (`email-model.ts` `emailTodo`).

## Older mail (backfill)

`email/backfill.rs`. After the first read (14 days), a background pass walks older Inbox and Sent mail, newest first, with the same filters, one batch of at most 60 emails per pass every 10 minutes (`BACKFILL_EVERY`).

- **Its own position:** `local_settings` `email.<provider>.<hash>.backfill` (`before`: the oldest time reached, `read`, `paused`, `done`). It never touches the change cursor. Outlook asks `GET /me/mailFolders/{inbox,sentitems}/messages?$filter=receivedDateTime lt <before> [and ge <floor>]&$orderby=receivedDateTime desc&$top=60`; Gmail `messages.list` with `q=before:<before> (in:inbox OR in:sent) -category:… [after:<floor>]`. A time cursor instead of page tokens: it survives restarts and needs nothing else.
- **Never competes:** a pass is skipped while an email check runs or Nandi has a run going or queued. A 429 or an outage skips the pass.
- **Range:** `EmailSettings.backfill` (`threeMonths`, `year`, `everything` by default, `off`), a floor in the queries. It's done when a batch comes back empty or the floor is reached.
- **Estimate:** `estimate_email_backfill(range)`: Graph `$count=true` with `ConsistencyLevel: eventual` per folder, Gmail `resultSizeEstimate`. Bulk isn't left out of the count, so it says "about".
- The batch says "older mail (backfill)": record owners, responsibilities and decisions; a passed deadline is history, not a to-do.
- `set_email_backfill(provider, pause|resume|stop|restart)`. Disconnect deletes the position.

## Where things live

A mailbox is its provider and its address (`MailboxId` in `email/provider.rs`). Its keys name it by `<provider>.<hash>`: the first 12 hex digits of the SHA-256 of the lowercased address, so keyring user names and `local_settings` keys never hold the address. Connecting a second address of the same provider adds a mailbox.

| What                                                 | Where                                                                                                                                   |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| A sign-in in progress (before the address is known)  | Keyring `nandi.email.<provider>.sign-in.<attempt>`, one per attempt (`EmailService.sign_ins`); connecting moves it to the mailbox entry |
| Sign-in tokens (access, refresh, expiry)             | Keyring `nandi.email.<provider>.<hash>.oauth`                                                                                           |
| Accounts (email, name, paused), interval, watch list | `~/.nandi/config.json` `email`, in the order they were connected                                                                        |
| Which agent may read which mailbox                   | grants: `settings.yaml` or `~/.nandi/settings.local.yaml`, per provider for now                                                         |
| Change cursor                                        | `local_settings` `email.<provider>.<hash>.cursor`                                                                                       |
| Last check (time, count, run, error, needs sign-in)  | `local_settings` `email.<provider>.<hash>.state`                                                                                        |

**Disconnect** (`email::disconnect`) takes one mailbox: it deletes its keyring entry, its `local_settings` rows and its account. The provider's grants go with its last mailbox. **Reconnect** of the same address (after a sign-in ended) keeps the cursor and the mailbox's place in the list; another address is added and starts with the 14-day window. Pause, Resume and the background read act on one mailbox too. The first mailbox stays this Nandi's identity (`services::identity`).

**The status the UI reads** (`EmailStatus`): `providers` (what this build can connect, and whether any mailbox of it is connected) and `mailboxes`, one `MailboxStatus` per connected mailbox in the order they were connected (provider, address, name, profile, state, last check, background read, `is_identity`). Settings › Email (`MailboxRow`) shows one row per mailbox with its own Pause or Resume, background-read controls and Sign out, and **Add another account** per provider (`ConnectMailbox another`). The first sign-in locks the identity (`config.json` `identity`); it never moves. **Signing out the identity mailbox** is allowed and leaves the identity as it was: the other mailboxes don't inherit it, and the relay and the free trial ask for that address to be connected again (`NoIdToken::Disconnected`). The sign-out dialog says so.

**Sign-ins in progress.** Each `start_email_sign_in` makes its own keyring entry (`MailProvider::sign_in_entry(attempt)`) and `EmailService.sign_ins` remembers the provider's current one. Starting again replaces it (the old flow is cancelled and its entry deleted), so two attempts never share a token. If `connect` fails after moving the token (config.json can't be written), a new mailbox's token entry is deleted again: no secret is left without an account.

**From Nandi 0.2** (`email/keys_from_0_2.rs`): 0.2 kept one mailbox per provider under `nandi.email.<provider>.oauth` and `email.<provider>.{cursor,state,backfill}`. At start, before anything reads a mailbox, each moves to the name of the account `config.json` holds for that provider: new name written first, old one deleted after, so an interrupted start finishes the next time. `state.db`'s schema doesn't change, so there is no migration.

## Providers

Client IDs are baked in at build time with `option_env!` (`email/provider.rs`; `build.rs` reruns when they change). A build without one shows "Not available in this build" for that provider.

| Provider | Build variables                                        | Sign-in                                                                                   | Scopes                                                                |
| -------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| Outlook  | `NANDI_MICROSOFT_CLIENT_ID`                            | `common` authority, PKCE on `http://localhost:<port>` (any port), device code as fallback | `openid profile email offline_access User.Read Mail.Read`             |
| Gmail    | `NANDI_GOOGLE_CLIENT_ID`, `NANDI_GOOGLE_CLIENT_SECRET` | PKCE on `http://127.0.0.1:<port>/callback`, `access_type=offline`, `prompt=consent`       | `openid email profile https://www.googleapis.com/auth/gmail.readonly` |

The loopback redirect is per provider (`LoopbackRedirect`). For `localhost`, the redirect server listens on `127.0.0.1` and `::1` on the same port.

### Changes and cursors

| Provider | First read (or expired cursor)                                                                                                      | Next checks                                                       | Expired              |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | -------------------- |
| Outlook  | `GET /me/mailFolders/{inbox,sentitems}/messages/delta?$select=…&$filter=receivedDateTime ge <14 days>`, following `@odata.nextLink` | the stored `@odata.deltaLink` per folder                          | 410: read the window |
| Gmail    | `historyId` from `GET users/me/profile` first, then `GET users/me/messages?q=after:<14 days> (in:inbox OR in:sent) …`               | `GET users/me/history?startHistoryId=…&historyTypes=messageAdded` | 404: read the window |

Headers come from `$select` (Outlook) and `format=metadata` with named headers (Gmail). Outlook's list headers are read per message (`$select=internetMessageHeaders`), only for inbox mail that passed the cheap filters. Catching up after a long time still keeps only the last 14 days.

### Filters (before the model sees anything)

`email/filter.rs` holds only deterministic rules:

- Bulk: Outlook `inferenceClassification = other`; Gmail `CATEGORY_PROMOTIONS`, `SOCIAL`, `UPDATES`, `FORUMS`; any `List-Unsubscribe` or `Precedence: bulk|list|junk` (`mail::ListHeaders`). The user's sent mail is never bulk.
- Notes to self: from the user, to only the user.
- **Always include** (`email.watch`): senders (`lena@contoso.com`) or domains (`contoso.com`, subdomains too) that pass even when they look like bulk.

Everything else reaches Nandi with the facts it needs to judge it. Machine senders, mail the user never opened and phishing are the model's call, not a word list's (owner decision 9):

| Fact             | From                                      | On the batch line       |
| ---------------- | ----------------------------------------- | ----------------------- |
| `unread`         | Outlook `isRead`, Gmail `UNREAD`          | `unread`                |
| `list_id`        | `List-Id`, as sent (a team list, maybe)   | `list: <id>`            |
| `auto_submitted` | `Auto-Submitted` other than `no`, as sent | `auto-submitted: <val>` |
| `bulk`           | the rules above (watched senders only)    | `bulk`                  |

The check message says what each fact means and what phishing looks like (`batch::FACTS`); the `email-triage` playbook says the same for chats. The background read of older mail uses the same filter.

## Tokens and errors

`auth::oauth::fresh_access_token` returns the stored token while it's valid (60 s margin), else refreshes it (`grant_type=refresh_token`; a rotated refresh token replaces the old one). One refresh runs at a time. `MailApi::get` renews once on a 401.

| What happens                                                                         | Result                                                                      |
| ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| The provider refuses the refresh (`invalid_grant`; Google Testing mode after 7 days) | `TokenError::NeedsSignIn`: the card shows **Reconnect**; the cursor is kept |
| Sign-in server unreachable                                                           | `TokenError::Unavailable`: the next check tries again; the sign-in stays    |
| 429 / 503                                                                            | Wait for `Retry-After` (10 s at most), twice; else try next check           |
| A URL outside the provider's API                                                     | Refused: the token only goes to Graph, or Gmail and Google's userinfo       |
| Nandi can't take the run                                                             | The cursor isn't saved, so the next check hands the same mail over          |

Sign-in failures are sorted by `classify_sign_in_error` into `adminApproval` (AADSTS65001, 90094, 90095: shows the admin-consent link `https://login.microsoftonline.com/organizations/adminconsent?client_id=…`), `declined`, `network`, `expired` and `other`. The UI explains each in plain words and shows the provider's text below.

## The run

`email::check_all` builds the message with `batch::Batch::prompt`: who the user is (each connected account's name and address, Nandi's "me"), whether it's the first read, then one line per email (id, folder, the facts above, date, from, to, cc, subject, a 240-character preview) inside `<email-data>`. Each check has its own conversation (`email:<uuid>`), so mail never enters the user's chat history.

`orchestrator::llm_step` sees `source_type = email` (`is_email_run`) and offers only the notes tools (`filesystem__read_file`, `list_dir`, `write_file`), the mail tools and Nandi's work tools (`work_open`, `work_start`, `work_update`: it acts on what the mail calls for, see [the interaction layer](./interaction-layer.md#nandi-acts-on-what-arrives)), without starting the container. Tool dispatch refuses anything else in that run.

Nandi's behaviour is in its `skill.md` › Email: note asks, promises, deadlines, decisions and owners; cite "(from <sender>, <date>, "<subject>")"; never copy more than a short phrase; don't re-add what the user deleted.

### Tools

Offered to any agent with a read grant on a connected mailbox (`email::tool::offered`), in chats too:

| Tool               | Arguments                | Returns                                           |
| ------------------ | ------------------------ | ------------------------------------------------- |
| `mail_list_new`    | `since_hours?`, `limit?` | Recent inbox and sent mail, filtered              |
| `mail_get_message` | `id`                     | Text with its quoted replies, at most 4,000 chars |
| `mail_get_thread`  | `thread_id`              | The thread's headers and previews                 |
| `mail_search`      | `query`, `limit?`        | Matching headers                                  |

IDs carry their provider: `outlook:<id>`, `gmail:<id>`. Results are wrapped as untrusted data.

## Commands and routes

| Command               | Route                               |
| --------------------- | ----------------------------------- |
| `get_email_status`    | `POST /api/rpc/get_email_status`    |
| `start_email_sign_in` | `POST /api/rpc/start_email_sign_in` |
| `poll_email_sign_in`  | `POST /api/rpc/poll_email_sign_in`  |
| `check_email_now`     | `POST /api/rpc/check_email_now`     |
| `set_email_paused`    | `POST /api/rpc/set_email_paused`    |
| `disconnect_email`    | `POST /api/rpc/disconnect_email`    |
| `set_email_settings`  | `POST /api/rpc/set_email_settings`  |

`email://changed` fires on connect, disconnect, pause and every check; `useEmailStatus` refetches on it.

## When sign-in fails

The real reason is on the browser page ("Sign-in failed: <OAuth error>: <description>"), under the Email card, and in the app log (`log::warn!`): `%LOCALAPPDATA%\com.nandi.app\logs\nandi.log` on Windows, `~/Library/Logs/com.nandi.app/nandi.log` on macOS, `~/.local/share/com.nandi.app/logs/nandi.log` on Linux. Tokens never appear in any of them.

| What it says                                            | Why                                                                        | Fix                                                                                                                                                                                                                                            |
| ------------------------------------------------------- | -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AADSTS7000218 (`client_secret` required), AADSTS9002327 | The Entra app's redirect sits under **Web** or **Single-page application** | Entra admin center › **App registrations** › Nandi › **Authentication** › remove the Web/SPA `http://localhost` › **Add a platform** › **Mobile and desktop applications** › `http://localhost`; **Allow public client flows: Yes** › **Save** |
| AADSTS65001, 90094                                      | The company needs an admin to approve Nandi                                | The admin-consent link on the card                                                                                                                                                                                                             |
| "couldn't save your sign-in"                            | The keychain refused the value                                             | See the details; large sign-ins are split (`secrets.rs`), so this shouldn't happen for size                                                                                                                                                    |
| "couldn't read whose mailbox this is"                   | Signed in, but `/me` (Graph) or the Gmail profile failed                   | The details carry the API's answer                                                                                                                                                                                                             |

**Large sign-ins.** A personal Microsoft account's tokens are several KB, over Windows Credential Manager's 2,560-byte limit per credential. `services::secrets` splits any value over 1,280 UTF-16 units into `<entry>#1`, `<entry>#2`, … with a header in `<entry>`; the in-memory test store enforces the same limit.

## Local builds with sign-in

PowerShell, from the repo:

```powershell
$env:NANDI_MICROSOFT_CLIENT_ID = "c62fc612-7271-4856-8124-b224b381a3d5"
$env:NANDI_GOOGLE_CLIENT_ID = "<the Google client ID>"
$env:NANDI_GOOGLE_CLIENT_SECRET = "<the Google desktop client secret>"
pnpm tauri build --debug
```

CI's release build passes the repository variables `NANDI_MICROSOFT_CLIENT_ID`, `NANDI_GOOGLE_CLIENT_ID` and the secret `NANDI_GOOGLE_CLIENT_SECRET` (`.github/workflows/release.yml`).

## Tests

`services/email/tests.rs` runs the whole check against a mock Graph and Gmail on `127.0.0.1` (no network): delta paging and resume, 410 and 404 restarts, filters and the watch list, one run for two mailboxes, refresh, a 401, a refused refresh, sign-out, the tools, and that every request to a mail API is a `GET` under `/me`.

## Not yet

- Who the user is at first run (task-1 §3): the mail identity is used in the email runs only.
- Push instead of polling; IMAP; sending (with its own scope and approval).
