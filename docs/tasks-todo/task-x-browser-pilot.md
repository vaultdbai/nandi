# Task: Browser Pilot — a visible browser agents drive on the host

**Goal:** agents can open and drive a real, visible Chrome/Chromium window on the user's machine. The user watches it work. Examples: research a topic, fill a form, check a pricing page, verify a deployment. Every step goes through Nandi's consent, approval, policy, egress and audit layers.

**Size:** L (about 6–8 days across the phases; phases 1–3 about 3–4 days)

**First cut (owner, 2026-10-28):** a short version of phase 1 came first: Playwright MCP as two catalog entries, `browser` (visible) and `browser-headless`, with allowed origins taken from network rules. Headless must go through the egress proxy under the agent's identity, with policies applied, already in that cut. So the part of phase 2 that gives each agent its own browser and proxy route is done there for headless. The rest of this task builds on it.

## How it fits what exists

- **Pilots are the mechanism** (`src-tauri/src/services/pilots/`, `docs/developer/pilots.md`, catalog CP-07 to CP-10). Pilots are host-side MCP servers, and **the host is the only MCP client**. The orchestrator calls the pilot and gives the model `pilot_browser__<tool>` tools. Containers are never given a pilot's address. A direct connection would skip consent, approvals and the audit log, and since #9 containers can't reach the host anyway (internal `nandi-agents` network). So there is **no** container-side setup and no `BROWSER_PILOT_URL`.
- **Don't build a CDP stack.** Wrap a maintained browser MCP server as the pilot: Microsoft's Playwright MCP (`@playwright/mcp`) or Google's `chrome-devtools-mcp`. Verify features and pick one in phase 1. They are expected to provide:
  - a visible browser by default;
  - a separate or persistent `--user-data-dir`;
  - allowed and blocked origins;
  - tools that work on the page's accessibility tree, which models use more reliably than raw DOM or coordinates.

  Pin the version, as the harness pins its CLIs.

- **The catalog's browser-capable placeholders are gone** (OSWright and Desktop Pilot were never published; removed 2026-10-29). OpenDesk, a real desktop pilot, stays; it drives the whole desktop, not a browser profile per agent, so it doesn't replace this.

## Design

### Launch

- **Catalog entry** `browser` (platform `any`): the chosen MCP server, started with the npm package pinned to a version, and a dynamic port on 127.0.0.1. It gets new capabilities `browser.navigate`, `browser.interact` (click, type, select), `browser.read` (snapshot, screenshot, URL, title), `browser.download` and `browser.submit`.
- **One browser per agent, never shared between agents.** A shared browser mixes sign-ins and tabs when agents run at the same time.
- **Profile per agent, persistent:** `~/.nandi/local/browser-profiles/<agent_id>/`. It's never the user's everyday Chrome profile, and the agent never sees the user's cookies or saved passwords.
- **Acting as the user:** the user signs in once, inside that agent's profile, while watching, and the agent keeps the session. We don't offer "use my Chrome profile": Chrome locks a profile while it's open, and recent versions refuse remote control of the default profile.
- **Finding Chrome:** the system Chrome/Chromium in the usual paths per OS. If there's none, the MCP server's own install of a managed browser.
- **The browser closed by the user:** detect it, mark the pilot's browser for that agent closed, and show a toast. The next tool call reopens it.

### Network: through the egress proxy

A browser on the host would otherwise bypass the egress proxy, and an agent could leak data through query strings to sites it's allowed to visit. So:

- Launch the browser with the egress proxy (`--proxy-server=127.0.0.1:<proxy port>`) using **this agent's** proxy identity. The agent's `network:` rules, narrowed by policies (`services/policies.rs`), then decide which sites load. Browsing is logged in `egress.log` with all other traffic, and there is one place that enforces it.
- **No separate allow-list file.** The allowed sites are the agent's network rules (gold copy, policies apply). The MCP server's allowed origins are set from the same rules as a second line of defence. Wildcards use the existing `HostRule` matching. The Settings screen shows the effective list and links to where it's edited.
- `localhost` / `127.0.0.1` are allowed only if the agent's rules list them.

### Consent and approvals

- **Consent works as it does today.** Every capability is granted explicitly per agent, and nothing is granted by default. A "grant the recommended set" button grants `navigate`, `interact` and `read` in one step. `download` and `submit` are always separate grants.
- **Approvals that look at the call (new, and generic for every pilot):** a per-pilot classifier inspects each call's arguments and decides whether it needs approval, beyond today's per-tool rule. For the browser:
  - form submissions that send data (POST, PUT, DELETE)
  - file downloads
  - typing into password fields
  - URLs carrying `token=`, `secret=`, `key=` or similar
  - clicking a link that leads off the allowed sites (the click is held and the target shown)

  These become Inbox approvals (RQ-18), and skill.md `requires_approval` can add more.

- **Audit:** every call goes to `pilots.log` with redacted arguments, the URL, the outcome and the duration. Screenshots are saved as run files and linked from the run (RN-05), not stored in the log.

### Coding agents (phase 4)

Pilot tools reach only Nandi's own agents today; Claude Code, Codex and Gemini in the container can't use them. Give them the browser through Nandi: Nandi serves pilot tools to the CLI over ACP as an MCP server. Every call goes through the same consent, classifier, approval and audit path, and the container still never talks to the pilot directly.

## Files

| File                                                                   | Change                                                                                              |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `src-tauri/src/services/pilots/catalog.rs`                             | `browser` entry, new capabilities; remove placeholder entries that don't exist                      |
| `src-tauri/src/services/pilots/runtime.rs`                             | start one per agent, with profile dir, proxy and allowed origins; detect a closed browser           |
| `src-tauri/src/services/pilots/classify.rs` (new)                      | the generic per-pilot approval classifier, with browser rules                                       |
| `src-tauri/src/services/pilots/tools.rs`                               | route calls through the classifier; save screenshots as run files                                   |
| `src-tauri/src/services/egress/*`                                      | a proxy identity for an agent's host browser; source label `browser:<agent>` in `egress.log`        |
| `src/components/library/` (pilot drawer)                               | browser card: running indicator and Stop, effective allowed sites, profile clean-up, recent actions |
| `docs/developer/pilots.md`                                             | the browser pilot, the classifier, the proxy routing                                                |
| `docs/userguide/userguide.md` and `src-tauri/resources/studio-guides/` | how to use it, how to allow sites, how to sign in once in the agent's profile                       |
| `docs/testing/library-capabilities-studio.md`                          | new CP- entries per phase                                                                           |

## Phases

- [ ] **1. Pick and wrap:** choose Playwright MCP or chrome-devtools-mcp (features, licence, how stable its tool names are), add the catalog entry, start it per agent with a profile and port, health check, check the existing catalog entries are real.
- [ ] **2. Network and consent:** the browser goes through the egress proxy with the agent's identity, allowed origins come from network rules and policies, and the capabilities and "grant recommended set" button are added.
- [ ] **3. Approvals and audit:** the generic classifier with browser rules, Inbox approvals, screenshots as run files, audit fields.
- [ ] **4. Coding agents:** pilot tools served to the CLI over ACP, through the same path.
- [ ] **5. UI and examples:** the browser card and screens, plus starter-pack capabilities `research.web` and `verify.deployment`.

## Tests

- **Unit:** the classifier (each sensitive case, and the cases that aren't sensitive), mapping allowed origins from `network:` rules and policies, profile paths per agent, the capability checks.
- **Integration (Docker or browser needed, `#[ignore]`):** launch, open a local fixture page, take a snapshot and a screenshot, close.
  - A site not on the agent's rules is refused, both by the proxy (and logged) and by the allowed origins.
  - A form POST waits for approval.
  - A coding agent's call goes through consent and audit.
- **UI:** the running indicator, the effective allowed sites list, profile clean-up.

## Acceptance

- [ ] From Library › Tools the user installs and starts the browser pilot, grants an agent its capabilities, and watches a Chrome window that agent drives.
- [ ] A site outside the agent's network rules or policies doesn't load. The tool result says why, and the attempt appears in `egress.log`.
- [ ] A form submission or download waits for approval in the Inbox.
- [ ] Every call is in the pilot audit log with redacted arguments, and screenshots are linked from the run.
- [ ] Two agents running at once each have their own browser and sign-ins.
- [ ] A Claude Code agent can use the browser through Nandi (phase 4).

## Out of scope

Firefox and Safari; the user's everyday Chrome profile; continuous recording or playback; a browser inside the container.

## Open questions for the owner

- Should Playwright MCP or chrome-devtools-mcp be the default? Is that decided in phase 1, or is there a preference?
- Should a browser start when an agent's run begins, or only on its first browser call?
- Should `localhost` be allowed by default for local development agents?
