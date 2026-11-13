# Connector Hub: Packages, Grants, Calendars and Files

Connectors anyone can build, publish and install; grants given while an agent works; read-only calendars and files beside mail; the Chief of Staff keeping My notes from the calendar. Mail itself is in [email.md](./email.md).

## Connector packages

A published connector is **one file**, `connector.md`: short frontmatter and plain instructions for agents. The format is in `nandi-spec/registry-schema/connector-md.md`; the registry entry made from it is `connector.schema.json`.

```markdown
---
name: Issue tracker
description: Read and file issues.
version: 1.0.0
run: npx -y @acme/tracker-mcp@1.2.3 # or uvx <pkg>==<version>, or url: https://…
auth: { type: api_key, env: [TRACKER_TOKEN], label: Tracker API key }
hosts: [api.tracker.example]
tools:
  - { name: list_issues, description: List open issues, access: read }
  - { name: create_issue, description: File an issue, access: write }
reads: [Issues in your projects]
writes: [New issues you ask for]
---

Use it to look up issues. Never file one without asking.
```

| Rule                                                                       | Where                                       |
| -------------------------------------------------------------------------- | ------------------------------------------- |
| Exact versions only (no ranges, `latest`, `x`; npm needs `x.y.z`)          | `domain/connector_package.rs` `RunSpec`     |
| Every host listed; a stdio server's container reaches only those           | `declaration()` → `config.network` → egress |
| Tools not listed count as **write**; write tools ask for each call         | `tool_access`, `safe_tools` = read tools    |
| `placement: host` is never installed from the public registry (decision 2) | `check_public`, `marketplace/connectors.rs` |
| Instructions at most 600 words; they reach an agent once it's granted      | `connectors::package_instructions`          |
| An HTTP server has no image and no placement                               | `ConnectorPackage::check`                   |

Installed, a package is two files in the gold copy: its declaration in `.nandi/connectors.yaml` (`kind: mcp`, `config.package {version, registry, publisher, commit}`) and `.nandi/connectors/<id>/connector.md`. An API key goes to the keyring as `nandi.connector.<id>.key` (`secret_refs` only in the file).

### The connectors image

stdio connectors run in `ghcr.io/vaultdbai/nandi-connectors:sandbox-6` (`runtimes/nandi-connectors`: Node LTS, Python, uv, the egress relay; no coding CLIs), pulled on first use like the other sandbox images (`images::SandboxImage::Connectors`). A connector with `image:` runs in that image (`docker pull` on first use); the install dialog says so. `images.yml` builds it on pushes to main.

### Registry, install and publish

```text
index.json connectors[] ─▶ Library › Browse › Connectors (marketplace/connectors.rs list)
Install ─▶ connector.md fetched at the entry's ref (a moved tag is refused) ─▶ must say exactly
  what the entry says (matches_entry) ─▶ consent dialog ─▶ connectors.yaml + connector.md + key
Studio › New connector ─▶ studio/connector.rs scaffold (tools listed by starting the server once)
Publish ─▶ tag <id>-v<version> ─▶ entry signed with ~/.nandi/keys/publisher.key ─▶
  GitHub registry: pull request (publish.rs, EntryList::Connectors); file:// registry: written in place
```

Signature: `nandi-signature-v1\nconnector\n<id>\n<version>\n<commit>\n` (`nandi sign --kind connector`). Packs list what they need in `.nandi/manifest.yaml` `requires_connectors: [id]`; the registry carries it on the pack entry (`PackManifest.requires_connectors`).

## Grants

One model for every connector that reaches the person's data: installed MCP connectors and the accounts (`outlook`, `gmail`, `outlook-calendar`, `google-calendar`, `onedrive`, `google-drive`). Built-ins (filesystem, web search, GitHub, AWS, S3 Backup) keep their skill.md declaration and install consent.

| Scope   | File                           | Travels           |
| ------- | ------------------------------ | ----------------- |
| `saved` | `.nandi/settings.yaml`         | yes, backed up    |
| `local` | `~/.nandi/settings.local.yaml` | no, this computer |

```yaml
connector_grants:
  - { agent: chief-of-staff, connector: outlook, access: read }
```

- Per agent and connector, `read` and `write` kept apart. Nothing expires (`domain/grant.rs`, `services/grants.rs`).
- Read on **every call** (`services/access.rs` `decide`), so removing one stops the next call mid-run.
- Connecting a mailbox, calendar or drive gives the Chief of Staff a saved read grant; disconnecting removes every grant on it.

### A call without a grant

```text
run_call ─▶ access_gate ─▶ decide(agent, tool)
  policy connectors.forbid matches   ─▶ refused (audit "forbidden")
  granted, or Allow once in this run ─▶ runs (mail/calendar/files: only the allowed accounts)
  else                               ─▶ approval tool `connector_access`, call id `<call>#access`
answer_access_request(choice) ─▶ grant (allowSaved | allowLocal) or not (allowOnce | deny)
  ─▶ choice written into the approval's args (the run record) ─▶ approve_action
```

- The request is an ordinary approval: it shows in the run, the Inbox and the companion, survives restarts and expires like any approval (expired = not allowed).
- `connectors.ask` in a policy asks every time: only Allow once and Don't allow (`askEveryTime`).
- Don't allow (or an expired request) isn't asked again in that run; the model is told to stop.
- Audit: each choice is a decision in `.nandi/decisions/<yyyy-mm>.md` (PO-14): the access request's entry names the connector and the choice (`approved (allowOnce)`, `refused (deny)`), a grant given or removed is kind `connectorGrant`, and a connector a policy forbids is kind `policy`, decided by it. `connectors.log` keeps only the calls themselves.
- A browser pilot opening a site outside the agent's allowed sites asks through the same approval, family `site` (no connectors; `args.site` holds the host). Its answers are Allow once (this run), Always allow for the agent (`allowSaved`: written into its skill.md) and Don't allow; `allowLocal` is refused. `answer_access_request` takes `subdomains` for it. See `docs/developer/pilots.md` › The browser pilots.

Policy:

```yaml
connectors: { forbid: [gmail, 'tracker:write'], ask: ['outlook*'] }
```

### Commands

| Command                                                               | Route                                                                |
| --------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `list_connector_grants`                                               | `POST /api/rpc/list_connector_grants`                                |
| `grant_connector_access`                                              | `POST /api/rpc/grant_connector_access`                               |
| `revoke_connector_access`                                             | `POST /api/rpc/revoke_connector_access`                              |
| `answer_access_request`                                               | `POST /api/rpc/answer_access_request`                                |
| `list_registry_connectors`                                            | `POST /api/rpc/list_registry_connectors`                             |
| `get_connector_consent`, `install_registry_connector`                 | `POST /api/rpc/get_connector_consent`, `…/install`                   |
| `remove_connector`, `scaffold_connector`, `publish_connector`         | `POST /api/rpc/remove_connector`, `/connectors/scaffold`, `/publish` |
| `get_workspace_status`, `start/poll_workspace_sign_in`                | `POST /api/rpc/{command}`                                            |
| `disconnect_workspace`, `check_calendar_now`, `set_calendar_interval` | `POST /api/rpc/{command}`                                            |
| `list_note_suggestions`, `answer_note_suggestion`                     | `POST /api/rpc/{command}`                                            |

Events: `grants://changed`, `workspace://changed`, `connectors://changed`.

## Calendars and files

`services/workspace/`: our own connectors on the shared sign-in (`auth/oauth.rs`) and the GET-only client (`email/api.rs` `MailApi::for_account`). Tokens stay in the keyring as `nandi.<id>.oauth`; tools run on the host.

**Incremental consent.** Each account signs in separately with only its own scopes, so mail, calendar and files are asked for when the person connects each one. The Entra app already has these delegated permissions.

| Account          | Scopes (plus `openid profile email`, Microsoft also `offline_access User.Read`) | Change feed                                                          |
| ---------------- | ------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Outlook calendar | `Calendars.Read`                                                                | `GET /me/calendarView/delta` (yesterday to +14 days), then deltaLink |
| Google Calendar  | `https://www.googleapis.com/auth/calendar.readonly`                             | `GET calendars/primary/events?timeMin`, then `syncToken`             |
| OneDrive         | `Files.Read`, `Files.Read.All`                                                  | `GET /me/drive/root/delta?token=latest`, then deltaLink              |
| Google Drive     | `https://www.googleapis.com/auth/drive.readonly`                                | `GET changes/startPageToken`, then `changes?pageToken`               |

Cursors (`local_settings` `workspace.<id>.cursor`) restart on 410 and weekly (the calendar window moves). Free/busy comes from `calendarView` / `events` (no POST). File text: text, Markdown, CSV, JSON and Google Docs (`export`), at most 12,000 characters; other files are linked, never copied.

| Tool                                            | Returns                                                                     |
| ----------------------------------------------- | --------------------------------------------------------------------------- |
| `calendar_events {start?, end?, limit?}`        | events, soonest first (default the next 7 days)                             |
| `calendar_get_event {id}`, `calendar_free_busy` | one event with 2,000 characters of description; busy and time-off intervals |
| `files_recent`, `files_search {query}`          | files with links (OneDrive search includes shared files)                    |
| `files_read_text {id}`, `files_changes`         | a document's text; what changed since the last call                         |

Offered to an agent with a grant on a connected account, or that names the account in skill.md `connectors:` (then the first call asks). IDs carry the account: `outlook-calendar:<id>`.

## The Chief of Staff and the calendar

Every hour by default (`set_calendar_interval`, 15 minutes to 24 hours), `workspace::check_calendars` reads each granted calendar's feed and hands every change to the Chief of Staff in **one** run (source `calendar_check`, conversation `calendar:<uuid>`), only when there is something. The run gets only the notes tools, `calendar_*` and the Chief of Staff's work tools (`orchestrator::data_run_allows`), and events sit in `<calendar-data>` (untrusted). Its skill.md › Calendar keeps `meetings.md`, `time-off.md` and `suggestions.md`.

**Suggestions and corrections** (`notes/corrections.rs`): `- [ ] …` lines in `suggestions.md` are accepted (ticked) or dismissed (removed) on My notes; both are remembered in `corrections.md`. Saving a note from My notes that drops or changes a cited line (`(from …)`, `(calendar…)`) adds "Removed from <note>: …" there. The skill.md reads `corrections.md` first and never adds those back.

## Tests

| What                                                                                                                        | Where                                  |
| --------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| Studio scaffold → publish to a file:// registry → install on a second Nandi → pause → Allow → revoke → Don't allow → policy | `src-tauri/src/connector_hub_suite.rs` |
| Allow once lasts for the run                                                                                                | same                                   |
| Mock Graph and Google: identity, grant, one run per calendar check, cursors, tools, GET only                                | `services/workspace/tests.rs`          |
| Format, entry, declaration, grants, policies, suggestions                                                                   | unit tests in each module              |
