# Pilots

Pilots are host-side MCP servers that let an agent control the OS: apps, files, the browser, system settings. The code is in `src-tauri/src/services/pilots/`, and the UI is the pilot cards and drawer in the Library (`components/library/`, kind Tools), `PilotConsentDialog`, `PilotConsentPrompt`, and the Pilots section of `PermissionsPanel`.

## Who can use a pilot

An agent gets a pilot's tools only when all of these hold (`tools::access`):

1. Its skill.md `pilots:` block lists the pilot.
2. The pilot is installed, and was started by this app session. A browser pilot needs no Start: the agent's first use turns it on (see below).
3. The user granted the agent **every** capability the pilot exposes.
4. Its last health check passed. Checks run every 30s with a 2s timeout. A browser has no shared server to check: a failed start is a warning for that step.

Nothing installs on its own. Grants are per agent and per pilot.

**Installed some other way counts.** The `installed` flag is set when Nandi runs the install. Before saying "not installed", `detect.rs` looks for the pilot where its install would put it: `npm ls -g --json` for `@playwright/mcp` at the pinned version (`browser`), `docker image inspect` for the pinned image (`browser-headless`). Each catalog entry's probe is `catalog::probe`. A find is recorded as installed, logged ("Found pilot ... already installed") and announced. Looking is read-only, and a miss is remembered for 60s so a run's steps don't each run npm. `pilot_status`, `start_pilot` and every run step look.

| ✅ Do                                            | ❌ Don't                                       |
| ------------------------------------------------ | ---------------------------------------------- |
| Check `pilots::installed` (flag, then the probe) | Trust only the `installed` row                 |
| Install only on the user's click or approval     | Install when the probe misses                  |
| Offer declared pilot tools directly every step   | Leave them to `tool_search` (deferred loading) |

**Pilot tools are never deferred.** `tool_search::is_core` keeps every `pilot_*` tool in the step's list. Playwright MCP alone has 20-odd tools: deferred, the agent saw only its file tools and didn't know to search for the browser.

**A browser that isn't installed offers one tool**, `pilot_<id>__setup` (`tools::SETUP_TOOL`), next to the `agent://warning`. The agent calls it when the task needs the browser. The person sees one approval card ("Set up the Browser pilot", button **Install**; `approval.summary.pilotSetup`). Approving runs the install (`tools::setup`), and the browser's tools come on the next step. Declining fails the call, and the tool's description tells the agent not to ask again. The agent never has to tell the person to go to Library › Tools.

## The host is the only MCP client

The orchestrator talks to each pilot from the host and gives the model its tools as `pilot_<pilot_id>__<tool>`. Containers are never given a pilot's URL. A direct connection would skip consent, the approval flow, and the audit log.

Every pilot tool needs approval unless the pilot marks it `readOnlyHint`. Every call is written to `~/.nandi/local/logs/pilots.log`, with secret-looking arguments redacted. The Audit Log view's Pilots tab shows it.

## Consent

Grants are machine-local state in SQLite `local_settings`: `consent.<agent_id>.<pilot_id>.<capability> = true`. They aren't part of the agent's definition.

| Action      | Command / route                                                               |
| ----------- | ----------------------------------------------------------------------------- |
| Read grants | `agent_pilot_permissions(agent_id)` / `POST /api/rpc/agent_pilot_permissions` |
| Grant       | `grant_pilot_capability(...)` / `POST /api/rpc/grant_pilot_capability`        |
| Revoke      | `revoke_pilot_capability(...)` / `POST /api/rpc/revoke_pilot_capability`      |

Grant and revoke take a `{"capability": "..."}` body over HTTP. A revoke takes effect on the agent's next step. `tools::call` re-checks access on every call.

Consent is asked for in three places:

- **On first use.** A run that leaves out a declared, running pilot only because consent is missing emits `pilot://consent_needed { agentId, pilotId, capabilities }` on its first step, next to the `agent://warning`. `PilotConsentPrompt`, mounted once in `App.tsx`, opens the consent dialog. If the user dismisses it, it stays quiet for that agent and pilot until the app restarts. Without that, an agent on a trigger would ask on every run.
- **From the Pilots view**, for any agent.
- **From the agent's Permissions tab.** Each capability has a switch, which grants or revokes it.

The run that triggered the prompt goes ahead without the pilot. The agent's next run gets its tools.

## The browser pilots

`pilots/browser.rs`. Two catalog entries, both Microsoft's Playwright MCP (`@playwright/mcp@0.0.83`), unmodified:

- **`browser-headless`**: Microsoft's image `mcr.microsoft.com/playwright/mcp:v0.0.83` (Install pulls it), run with `docker run -i` on the internal `nandi-agents` network. Its only way out is the egress relay.
- **`browser`**: a window on this computer, so the user can watch. `npx -y @playwright/mcp@0.0.83` drives the system Chrome (else Edge). Install runs `npm install -g` so the first start doesn't wait for a download.
  - It's the one exception to "agents run only in the container" (owner, 2026-10-30). The agent's own code still runs in its container. Only the browser it drives runs on the host, and it goes through the same egress proxy, allowed-sites check and per-call approvals as `browser-headless`. When watching isn't needed, prefer `browser-headless`.

**One browser per agent**, started on the agent's first use and kept running (`POOL`, keyed by agent and pilot). The first step of an agent with the pilot granted turns it on (`pilots::turn_on_browser`: checks the prerequisites, Docker and the image or npx and Chrome or Edge), so Start in the Library is optional. Stop shuts every agent's browser; the next agent that uses it opens one again. The status of a browser comes from its rows for this session, not the registry (it has no process of its own there). Changed settings (the agent's options or secrets) restart that agent's browser. Both talk to Nandi over stdio (`connectors::LineTransport`), not a port.

**Only the agent's sites load.** Everything is Playwright MCP's own config file, written per start and deleted once the server has read it:

| Setting                                                                       | Why                                                                                                                                                                                                                                                                |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `launchOptions.proxy`: the egress proxy, user `nandi`, password a proxy token | The proxy authenticates the browser as `<pilot>:<agent>` (`browser::identity`) and applies that agent's `network.allow`, narrowed by its policies, to every request the page makes. Each is in `egress.log` under that name. The token is never on a command line. |
| `bypass: <-loopback>` and `--proxy-bypass-list=<-loopback>`                   | Chromium skips the proxy for localhost unless told not to.                                                                                                                                                                                                         |
| `--force-webrtc-ip-handling-policy=disable_non_proxied_udp`                   | WebRTC could otherwise reach the network around the proxy.                                                                                                                                                                                                         |
| `userDataDir`: `~/.nandi/local/browser/<agent>/<pilot>/profile`               | Sign-ins persist per agent. Never the user's own browser profile.                                                                                                                                                                                                  |
| `outputDir` and the working folder: the agent's work folder                   | Screenshots are files (`browser/screenshot-….png`) the run's Files tab lists.                                                                                                                                                                                      |
| `secrets`                                                                     | The agent types a secret's name; Playwright MCP types the value and replaces it with `<secret>NAME</secret>` in every reply. Values come from the keychain.                                                                                                        |
| `imageResponses: omit`, `codegen: none`                                       | The model reads the accessibility tree, not images, and no generated code.                                                                                                                                                                                         |

Before a navigation, Nandi checks the URL against the same rules and refuses with "<host> isn't one of this agent's allowed sites" (`allowed_url`), so the agent can say why.

**A site outside the rules asks the person first** (`pilots/sites.rs`, through `services::access`, family `site`). `browser_navigate` and `browser_tabs` (`new`) to such a host pause the run with an ordinary access request, the same approval as a connector without a grant (see `docs/developer/connector-hub.md` › A call without a grant):

```text
navigate(url) ─▶ access_gate ─▶ decide ─▶ site_decision
  host allowed (network.allow)                       ─▶ runs
  not askable (IP, localhost, *.local, *.internal, a port, not http(s)) ─▶ refused by allowed_url, as before
  a policy's network ceiling leaves it out (`policies::admits_hosts`)  ─▶ refused, as before
  allowed once earlier in this run                    ─▶ runs (sites::allow_for_run again)
  refused or expired earlier in this run              ─▶ refused, not asked again
  else ─▶ `connector_access` approval, args.site = {host, display, pilot, reason, canSave}
answer_access_request(choice, subdomains)
  allowOnce  ─▶ sites::allow_for_run(agent, run): the browser's check and the egress proxy
                (`browser:<agent>` identity) add the host until the run ends (`EventBus::turn_done` ─▶ sites::end_run)
  allowSaved ─▶ Agent::allow_sites ─▶ agents repository save + commit (skill.md `network.allow`); only for the user's own agents (`canSave`)
  deny       ─▶ the call fails: "<site> isn't allowed: the person didn't let this agent open it…"
```

- `host` is what the browser sends (punycode); `display` is the decoded name the card shows next to it.
- A grant covers the host only. `subdomains: true` adds `*.host` too; the card's checkbox is off by default.
- Navigation tools get an optional `reason` argument (`with_reason`); Nandi strips it before Playwright sees the call and shows it on the card.
- Pages a site loads from other hosts aren't asked about: the proxy refuses them, as before. Ask for those hosts in the agent's Internet access. Playwright's own `allowedOrigins` isn't used: its docs say it isn't a security boundary, and it has no `*.domain` patterns, so it would disagree with the network rules.

**Approvals are decided per call** (`browser::action`, `needs_approval`). Playwright asks for each element's human-readable description so a client can decide on permission; that, the tool and its arguments give the kind:

| Kind     | Examples                                                                                | Asks unless skill.md allows |
| -------- | --------------------------------------------------------------------------------------- | --------------------------- |
| Read     | snapshot, screenshot, find, wait                                                        | never asks                  |
| Navigate | open an allowed page, back, tabs, close                                                 | never asks                  |
| Interact | click, type, select, hover, drag                                                        | `interact`                  |
| Submit   | a submit/send/save/delete/sign-in button, Enter, typing with submit, accepting a dialog | `submit`                    |
| Secret   | typing a secret's name, or into a password field                                        | `secrets`                   |
| Upload   | uploading files, a download or export button                                            | `upload`                    |
| Pay      | a pay/buy/checkout/place-order button                                                   | `pay`                       |

The arbitrary-code tools (`browser_evaluate`, `browser_run_code_unsafe`) are never offered.

**Hand-over** (`pilots/handover.rs`). The visible `browser` also offers `browser_hand_over {site, summary, draft}`, Nandi's own tool, for "prepare, a person finishes" (posting on social sites, sending a form the agent may not submit):

| Step      | What happens                                                                                                                                                                                               |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Asking    | Always an approval (`Action::HandOver`; skill.md can't allow it), kept 24 h (`handover::TTL_MS`). `run_call` brings the window forward (`browser::bring_to_front`: the current tab, `browser_tabs select`) |
| Answering | `answer_hand_over(approval_id, {outcome, finalText})` / `POST /api/rpc/answer_hand_over`: Done and Changed approve, Skip rejects. `HandOverCard` in every approval surface                                 |
| Result    | `tools::call` answers it with `handover::result`: what the person did, and the final text for Changed. A plain Approve counts as Done                                                                      |

✅ Fill the composer, then `browser_hand_over`. ❌ Click Post, press Enter or Ctrl+Enter to send.

**Refused controls** (`pilots/refusal.rs`, policy `browser.refuse`, see `domain/browser_rule.rs`). Before a click, key, typing, form, dialog, drag or upload, when a policy refuses controls for the agent, Nandi asks the page: `browser_snapshot {target, depth: 1}` gives the URL and the control's own `role "name"`; with no target, the tab list gives the URL. A rule refuses by site, then by the control's name (clicks and drags only; fields are never controls) or by kind (the classifier above; Enter and Ctrl+Enter are submits). The agent's description can add to a refusal, never exempt one: only the page's name matches `except`. The last URL each browser showed (`PAGES`) decides before a call runs whether it's refused, so a refused call isn't put to the person for approval first; if the page changed in between and the call needs approval, it's refused with "the page changed". Refusals are recorded in the decisions file (`.nandi/decisions/<yyyy-mm>.md`, kind `policy`), decided by the policy, as `browser controls`.

Packs can ship such policies for their own agents (`policies/<id>.md`; the installer refuses any other target). They load from the vendored pack as `<pack>/<id>` and aren't editable. Nandi Social's `social-posting` policy is the example.

**skill.md:** a `pilots:` item can be a mapping:

```yaml
pilots:
  - id: browser-headless
    allow: [interact, submit] # auto mode: these run without asking
    values: { email: dev@acme.test } # plain values; the agent reads them
    secrets: { SITE_PASSWORD: nandi.pilot.shopper.site-password } # keychain entries
```

The values and secret names go into the system prompt ("What your browser may type", `PilotToolset::prompt_notes`) and the descriptions of the typing tools and `browser_navigate`. On the tools alone the model treated them as documentation and asked the user for values it had. The secret's value is saved from the agent's Permissions tab (`set_pilot_secret`, write-only); the agent never sees it. Unknown keys, an unknown `allow` word or a secret name that isn't `CAPITALS_AND_DIGITS` are errors in skill.md. Saving the agent from its Settings tab keeps a pilot's mapping.

### Sites and accounts

The agent's Permissions tab shows, under each browser pilot (`AgentPilotPermission.browser`), the agent's sites and accounts:

| What                 | How                                                                                                                                                                                                                                                                                                                             |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Each secret's status | `secrets: [{name, isSet}]`, read from the keychain. Never the value.                                                                                                                                                                                                                                                            |
| Set, replace, remove | `set_pilot_secret` (empty removes). Write-only.                                                                                                                                                                                                                                                                                 |
| Add a secret         | `add_pilot_secret`: `Agent::add_pilot_secret` writes `secrets: {NAME: nandi.pilot.<agent>.<name>}` (`capability::pilot_secret_ref`, `_` becomes `-`) through the agent repository and commits, then the value goes into the keychain. Agents made on this computer only (the Chief of Staff too); a pack's agent names its own. |
| Sign out everywhere  | `clear_browser_profile`: `browser::sign_out` stops that agent's browser (token revoked, pages forgotten) and deletes `~/.nandi/local/browser/<agent>/<pilot>/profile`, nothing else. `hasProfile` says whether there is one.                                                                                                    |

Values never reach the model, a log, an event or the frontend: the prompt lists names only (`tools::fill_note`), Playwright MCP hides typed values in its replies and Nandi masks them again (`browser::mask_secrets`, the values held in memory with the running browser). `pilots::tests::a_site_secret_s_value_stays_in_the_keychain` checks the file, the Permissions view, events, the prompt note and every log line.

Profiles live under `~/.nandi/local`, never the repo, and the S3 backup leaves them out (`browser::profile_backup_exclude`, next to the screenshots' exclude).

Non-secret details of a site go in the agent's notes, `my-notes/private/sites/<site>.md` (the Chief of Staff's `sign-in-and-accounts` playbook). The filesystem connector refuses a write into `my-notes` that holds a Luhn-valid 13 to 19 digit run or a `password:` style line (`domain::note_guard`); a capitalised secret name (`Password: ACME_PASSWORD`) is fine.
