# Pilots

Pilots are host-side MCP servers that let an agent control the OS: apps, files, the browser, system settings. The code is in `src-tauri/src/services/pilots/`, and the UI is the pilot cards and drawer in the Library (`components/library/`, kind Tools), `PilotConsentDialog`, `PilotConsentPrompt`, and the Pilots section of `PermissionsPanel`.

## Who can use a pilot

An agent gets a pilot's tools only when all of these hold (`tools::access`):

1. Its skill.md `pilots:` block lists the pilot.
2. The pilot is installed and was started by this app session.
3. The user granted the agent **every** capability the pilot exposes.
4. Its last health check passed. Checks run every 30s with a 2s timeout.

Nothing installs or starts on its own. Grants are per agent and per pilot.

## The host is the only MCP client

The orchestrator talks to each pilot from the host and gives the model its tools as `pilot_<pilot_id>__<tool>`. Containers are never given a pilot's URL. A direct connection would skip consent, the approval flow, and the audit log.

Every pilot tool needs approval unless the pilot marks it `readOnlyHint`. Every call is written to `~/.nandi/local/logs/pilots.log`, with secret-looking arguments redacted. The Audit Log view's Pilots tab shows it.

## Consent

Grants are machine-local state in SQLite `local_settings`: `consent.<agent_id>.<pilot_id>.<capability> = true`. They aren't part of the agent's definition.

| Action      | Command / route                                                                |
| ----------- | ------------------------------------------------------------------------------ |
| Read grants | `agent_pilot_permissions(agent_id)` / `GET /api/agents/{id}/pilots`            |
| Grant       | `grant_pilot_capability(...)` / `POST /api/agents/{id}/pilots/{pilot}/grant`   |
| Revoke      | `revoke_pilot_capability(...)` / `POST /api/agents/{id}/pilots/{pilot}/revoke` |

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

**One browser per agent**, started on the agent's first use and kept running (`POOL`, keyed by agent and pilot). Start in the Library checks the prerequisites (Docker and the image, or npx and Chrome or Edge) and turns the pilot on; Stop shuts every agent's browser. Changed settings (the agent's options or secrets) restart that agent's browser. Both talk to Nandi over stdio (`connectors::LineTransport`), not a port.

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

Before a navigation, Nandi checks the URL against the same rules and refuses with "<host> isn't one of this agent's allowed sites" (`allowed_url`), so the agent can say why. Playwright's own `allowedOrigins` isn't used: its docs say it isn't a security boundary, and it has no `*.domain` patterns, so it would disagree with the network rules.

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

**skill.md:** a `pilots:` item can be a mapping:

```yaml
pilots:
  - id: browser-headless
    allow: [interact, submit] # auto mode: these run without asking
    values: { email: dev@acme.test } # plain values; the agent reads them
    secrets: { SITE_PASSWORD: nandi.pilot.shopper.site-password } # keychain entries
```

The values and secret names go into the system prompt ("What your browser may type", `PilotToolset::prompt_notes`) and the descriptions of the typing tools and `browser_navigate`. On the tools alone the model treated them as documentation and asked the user for values it had. The secret's value is saved from the agent's Permissions tab (`set_pilot_secret`, write-only); the agent never sees it. Unknown keys, an unknown `allow` word or a secret name that isn't `CAPITALS_AND_DIGITS` are errors in skill.md. Saving the agent from its Settings tab keeps a pilot's mapping.
