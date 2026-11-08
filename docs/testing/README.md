# Feature Test Catalog

The master list of every feature Nandi supports today, written so each one can be tested on its own, by hand now and by automated tests later. When this catalog and another doc disagree, the code is the authority. Fix whichever of the two is wrong.

Built on 2026-10-22 from the code at `15190d5`, `docs/developer/`, `docs/userguide/`, `docs/tasks-done/`, the status reports and `git log`. Updated on 2026-10-23 for #9 (`d9af777`, network isolation and the egress relay) and #10 (`04226b8`, policy files and the evaluator), and for the fixes to ON-07, SH-25, SH-26, SH-30, WF-26, WF-27, CP-16 and PL-10. Updated again on 2026-10-23 for #11 to #15 (`e2ffad3` to `2c7e231`: policies enforced on every run, shown on the agent page and edited in Studio, stdio connectors in their own container, `models.allow` for coding agents' CLI models, agent `tags`), and for the policy watcher (PO-11, DT-13), Publish's Back up now wording (LB-18) and new tests for PO-10, PO-13 and CP-22.

## Start here: a test session

### 1. Get a clean, current build

1. `git pull` on `main`, then `pnpm install`.
2. `pnpm run check:all` should pass. The same gates run in CI (`.github/workflows/ci.yml`) on every push to `main` and on every PR. Rust is pinned in `rust-toolchain.toml`.
3. **Local database:** if the app stops at start-up with "The local database is from another Nandi build", quit Nandi and delete `~/.nandi/local/state.db` and its `-wal`/`-shm` files. Agents, workflows, triggers and settings are in `~/.nandi/repo` and come back. Run history and the Inbox start empty. PowerShell: `Remove-Item "$env:USERPROFILE\.nandi\local\state.db*"`.
4. `pnpm run tauri:dev` starts the app.

### 2. Get the machine ready

| Needed for                                       | Set up                                                                                                                                                                                                                 |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Agents that use tools, coding agents, connectors | Docker Desktop running. Nandi downloads the sandbox images and Claude Code the first time an agent needs them (RT-01 to RT-03, RT-18, RT-29); **Download now** in Settings › Models & runtimes gets them ahead of time |
| Anything that calls a model                      | A provider key in Settings › Models & runtimes (MD-01), the free trial (MD-13), Ollama (MD-12) or the Claude sign-in (MD-10)                                                                                           |
| Coding agents                                    | The CLI's sign-in in Settings › Models & runtimes › Coding agents (RT-09)                                                                                                                                              |
| Backup, teams, publishing                        | A git remote you can push to (Settings › Backup, DT-18)                                                                                                                                                                |
| Network, delegation, gateway                     | A relay (`nandi-relay`, RL-07) and a second instance: `NANDI_HOME` set to another folder, one machine is enough (RL-11)                                                                                                |
| Throwaway testing                                | `NANDI_HOME` pointing at an empty folder, so your real `~/.nandi` is untouched. The OS keyring is shared (DT-34)                                                                                                       |

### 3. Run the quick automated checks first

- `pnpm run check:all`: all `unit` coverage.
- `pnpm smoke:api`: starts Nandi headless on a throwaway home and calls every parameterless GET route of the HTTP API. Every route should answer 200, and a call without the token 401. Build the binary first (`pnpm run build`, then `cargo build --bin nandi` in `src-tauri`).

### 4. Then test by hand, area by area

Go in this order. Each step needs what the steps before it set up:

1. **Shell, onboarding and settings** (`ON`, `SH`, `ST`): fresh home, onboarding, every Settings section.
2. **Data and platform** (`DT`, `PL`): the repo, backup, CLI, single instance.
3. **Runtimes, models and sandbox** (`RT`, `MD`, `SB`): keys, images, containers, the egress proxy.
4. **Agents, chat and runs** (`AG`, `CH`, `RN`): create an agent, chat, tools, approvals, run detail.
5. **Policies** (`PO`): write policies and watch what each agent ends up with.
6. **Workflows, triggers, requests and Inbox** (`WF`, `TR`, `RQ`).
7. **Library, capabilities, Studio and companion** (`LB`, `CP`, `AI`).
8. **Teams, network, delegation and relay** (`TM`, `NW`, `RL`), then the **gateway** (`GW`), with two instances.

Inside an area, entries marked `unit` and `api` are already covered by automation, so do a quick check by hand. Spend the time on `ui-e2e` and `manual`.

### 5. Record results and report problems

- In the [master index](#master-index), fill **Last verified** with `YYYY-MM-DD pass` or `YYYY-MM-DD FAIL: <short note>`. `pnpm catalog:index` keeps what you wrote.
- When an entry is wrong (the app does something else and the app is right), fix the entry: its Steps, Expected or Status.
- When the app is wrong, note the ID, the step, what happened and the relevant log:
  - the app's terminal output;
  - `~/.nandi/local/logs/llm.log`, `egress.log`, `pilots.log`, `triggers.log`;
  - Settings › Logs.

  Fix it, or add it to `docs/tasks-todo/`.

## How to use it

- **Test one feature:** find its ID in the [master index](#master-index), open its area file and follow **Steps**, then compare with **Expected**. Try the **Edge / negative cases** too. Record the date and the result in the **Last verified** column.
- **Run a full pass:** go through the files in the order of the table below. Start with entries whose layer is `unit` or `api`, which need no setup. Then do `ui-e2e`, and leave `manual` for last because those need Docker, a real model key, a second machine or an OS dialog.
- **Use a throwaway profile:** set `NANDI_HOME` to an empty temp folder so tests never touch your real `~/.nandi`. The OS keyring (service `com.nandi.app`) is **not** isolated by `NANDI_HOME` (see DT-34), so provider keys and the backup credential are shared with your real profile.

## Entry format

Each feature is a `### <ID> — <name>` heading followed by these fields:

| Field                     | Meaning                                                                                                                                                                                                                                                                                  |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **What**                  | The behavior a user can see                                                                                                                                                                                                                                                              |
| **Where**                 | UI path, key files, Tauri command, HTTP route, CLI flag                                                                                                                                                                                                                                  |
| **Preconditions**         | Setup the test needs                                                                                                                                                                                                                                                                     |
| **Steps**                 | Numbered steps that can be repeated                                                                                                                                                                                                                                                      |
| **Expected**              | Observable results: UI, files under `~/.nandi`, DB rows, events, log lines                                                                                                                                                                                                               |
| **Edge / negative cases** | Other cases to try                                                                                                                                                                                                                                                                       |
| **Automation**            | `Covered by:` existing tests, then `Layer:` the cheapest layer that can automate it: `unit` (vitest / cargo test), `api` (Tauri command or the HTTP/WS server, see RL-26), `ui-e2e` (needs the window), `manual`. Ends with `Gap:` for what is still untested                            |
| **Needs relay**           | `yes` when the feature involves another person's Nandi or the relay itself (asking or assigning someone, the team inbox, reassignment, handover, presence, the Connection settings). `no` for everything that works alone. Duplicate pointers carry the value of the entry they point to |
| **Status**                | `Shipped`, `Partial (<why>)`, `Known issue (<ref>)`, `Behind flag (<name>)` or `Removed (<date or commit>, <why>)`                                                                                                                                                                       |

The **HTTP API** (RL-26) is the main way to automate tests: run `nandi --server` or `--headless` (PL-02, PL-03) and nearly every feature can be driven over `/api/*` plus the `/ws` event stream, with no window.

## Keeping it current (required)

This catalog is only useful if it matches the code. Every change that adds, changes or removes user-visible behavior must update it **in the same change**:

1. **New feature:** add an entry to the matching area file using the next free number for that prefix, then run `pnpm catalog:index` to rebuild the master index and the counts.
2. **Changed feature:** update Steps, Expected and Status. Leave the ID as it is.
3. **Removed feature:** keep the heading and set Status to `Removed (<commit>)`. IDs are never reused or renumbered, because test results and other docs refer to them.
4. **New tests:** add them to the entry's `Covered by:` and shrink its `Gap:` to match.
5. **Duplicates:** if one feature turns up in two areas, test it in one entry and turn the other into a `Duplicate:` pointer.
6. **Needs relay:** every entry has one (`pnpm catalog:index` fails without it). A `no` feature that breaks with no relay configured or reachable **blocks a release**. `pnpm run test:offline` covers the automated part: it boots Nandi with no relay (and with an unreachable one) and drives the `no` features that matter most; test the rest by hand with Network › Connection empty.
7. Each area file ends with **Gaps and planned**, which lists promised work that isn't built yet. When an item ships, move it into a real entry.

ID prefixes:

| Prefix         | Area                                                                                  | File                                                               |
| -------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `ON` `SH` `ST` | Onboarding, shell and navigation, settings                                            | [shell-onboarding-settings.md](./shell-onboarding-settings.md)     |
| `AG` `CH` `RN` | Agents, agent console, runs                                                           | [agents-chat-runs.md](./agents-chat-runs.md)                       |
| `RT` `MD` `SB` | Runtimes and containers, models and keys, sandbox and egress                          | [runtimes-models-sandbox.md](./runtimes-models-sandbox.md)         |
| `WF` `TR` `RQ` | Workflows, triggers, requests / work / Inbox                                          | [workflows-triggers-requests.md](./workflows-triggers-requests.md) |
| `LB` `CP` `AI` | Library and marketplace, capabilities / connectors / pilots, Studio / Ask / companion | [library-capabilities-studio.md](./library-capabilities-studio.md) |
| `TM` `NW` `RL` | Teams and trust, network, relay / delegation / HTTP server                            | [teams-network-relay.md](./teams-network-relay.md)                 |
| `PO`           | Policies (policy files and what an agent ends up with)                                | [policies.md](./policies.md)                                       |
| `GW`           | Instance gateway agent                                                                | [gateway.md](./gateway.md)                                         |
| `DT` `PL` `QA` | Data and gold copy, platform / CLI / release, quality gates                           | [data-platform-quality.md](./data-platform-quality.md)             |

## Docs that disagree with the code

These turned up while building the catalog. In each case the catalog follows the code. Fix the doc, or fix the code if the doc describes what was intended.

- **Sign-in and teams (moved to v2, see `docs/tasks-todo/task-x-v2-sign-in.md`):** the app has no sign-in as identity. The first-run gate reads the local identity in `NANDI.md`, and teams can be any git URL (auth was removed in `91c8fad`). The optional GitHub sign-in (`github-auth.md`, ST-32) only supplies a token for github.com repositories. `entra-setup.md` and user guide §3 and §9 ("Sync now", "Log out", Entra ID, GitHub-org teams) are out of date; the user guide cleanup is listed in that task.
- **Share branches:** the code names them `nandi/share-<slug>-<agent>`. The docs say `nandi/`.
- **Library:** `marketplace.md` describes an Installed tab, a Modified badge, "Update all" and a Registries tab. None of these exist. `registry-schema/README.md` ([`nandi-spec/registry-schema`](https://github.com/vaultdbai/nandi-spec/tree/main/registry-schema)) still says "Marketplace → Registries", and Publish is in the agent's ⋯ menu, not its header.
- **Triggers:** `triggers.md` describes Definition/Fires/Payload tabs. The code has Settings and Firings tabs.
- **User guide:** "Work → Waiting on me" is now Home's Inbox (there is no Work page), and the "Memory tab" is under Settings › Advanced.
- **Delegation:** the Relay screen with "Call" is gone, and calls now go from Library → Shared. There is no delegation history page: delegations show in Home → All activity.
- **Data model:** "Sync now" (`syncAgentsRepo`) is still described in user guide §5 and §12, and in comments in `services/repos/git.rs`, `repo_service.rs`, `commands/repo.rs` and `team/registry.rs`, but it no longer exists. Publish's errors point to Settings › Backup › Back up now (LB-18).
- **Updater:** the updater reads the GitHub release's `latest.json` (`https://github.com/vaultdbai/nandi/releases/latest/download/latest.json`), which every release attaches. An update reaches people once its draft is published.
- **Template leftovers:** `tauri.conf.json` still has placeholder publisher, description and copyright. The identifier is now `com.nandi.app`.

## Summary

525 features to test: all 579 IDs, minus 43 duplicates that point to the entry where the feature is tested, minus 11 removed features.

| Area                                        | File                                                               | Features |
| ------------------------------------------- | ------------------------------------------------------------------ | -------- |
| App shell, onboarding and settings          | [shell-onboarding-settings.md](./shell-onboarding-settings.md)     | 57       |
| Agents, chat and runs                       | [agents-chat-runs.md](./agents-chat-runs.md)                       | 76       |
| Runtimes, models and sandbox                | [runtimes-models-sandbox.md](./runtimes-models-sandbox.md)         | 62       |
| Policies                                    | [policies.md](./policies.md)                                       | 16       |
| Workflows, triggers, requests and Inbox     | [workflows-triggers-requests.md](./workflows-triggers-requests.md) | 87       |
| Library, capabilities, Studio and companion | [library-capabilities-studio.md](./library-capabilities-studio.md) | 75       |
| Teams, network, delegation and relay        | [teams-network-relay.md](./teams-network-relay.md)                 | 72       |
| Instance gateway                            | [gateway.md](./gateway.md)                                         | 20       |
| Data, platform and quality gates            | [data-platform-quality.md](./data-platform-quality.md)             | 60       |

| Status      | Count |
| ----------- | ----- |
| Shipped     | 506   |
| Partial     | 16    |
| Removed     | 11    |
| Known issue | 3     |

| Best automation layer | Count |
| --------------------- | ----- |
| `unit`                | 279   |
| `api`                 | 185   |
| `manual`              | 37    |
| `ui-e2e`              | 24    |

| Needs relay | Count |
| ----------- | ----- |
| no          | 451   |
| yes         | 74    |

## Master index

Fill in **Last verified** as you test (`2026-10-22 pass`, `2026-10-22 FAIL: <short note>`). Details of each test are in the area file. `pnpm catalog:index` rebuilds this table and the Summary from the area files and keeps this column.

### App shell, onboarding and settings

| ID    | Feature                                                                  | Status                                               | Layer      | Needs relay | Last verified                          |
| ----- | ------------------------------------------------------------------------ | ---------------------------------------------------- | ---------- | ----------- | -------------------------------------- |
| ON-01 | First-run gate (AuthGate)                                                | Shipped                                              | api        | no          |                                        |
| SH-01 | Startup sequence                                                         | Shipped                                              | ui-e2e     | no          |                                        |
| SH-02 | Single instance                                                          | Shipped                                              | manual     | no          |                                        |
| SH-03 | Window size and position remembered; closing hides the window            | Shipped                                              | manual     | no          |                                        |
| ON-02 | Onboarding steps, step indicator and resume                              | Shipped                                              | api + unit | no          | 2026-11-01 pass                        |
| ON-03 | Step 2: your name and handle                                             | Shipped                                              | unit       | no          |                                        |
| ON-04 | Saving the identity (NANDI.md)                                           | Shipped                                              | api        | no          |                                        |
| ON-05 | Step 3: use a key already on this computer                               | Shipped                                              | unit       | no          |                                        |
| ON-06 | Step 3: enter and test a provider key                                    | Shipped                                              | api        | no          |                                        |
| ON-07 | Step 3: Ollama and Amazon Bedrock options                                | Shipped                                              | unit       | no          |                                        |
| ON-08 | Step 3: free tokens notice and skipping without a model                  | Shipped                                              | unit       | no          |                                        |
| ON-09 | Step 4: join a network (optional)                                        | Shipped                                              | unit       | yes         | 2026-11-01 pass after fix              |
| ON-10 | Step 4: "Let others send requests to my Nandi"                           | Shipped                                              | unit       | yes         |                                        |
| ON-11 | Step 5: starter packs                                                    | Shipped                                              | unit       | no          | 2026-11-01 pass                        |
| ON-12 | Finishing setup and the welcome strip                                    | Shipped                                              | unit       | no          |                                        |
| ON-15 | Step 1: connect your email (skippable)                                   | Shipped (live test pending)                          | unit       | no          |                                        |
| ON-13 | Starter packs page (after setup)                                         | Duplicate of LB-20                                   | —          | no          |                                        |
| ON-14 | Upgrades from pre-identity installs                                      | Removed (2026-10-22, no legacy support)              | —          | no          |                                        |
| SH-04 | Title bar and window controls                                            | Shipped                                              | ui-e2e     | no          |                                        |
| SH-05 | Title-bar bull (status at a glance)                                      | Shipped                                              | unit       | no          |                                        |
| SH-06 | Account menu                                                             | Shipped                                              | unit       | no          |                                        |
| SH-07 | Panes and panel toggles                                                  | Shipped                                              | unit       | no          | 2026-10-30 pass                        |
| SH-08 | Sidebar: the seven pages                                                 | Shipped                                              | unit       | no          |                                        |
| SH-09 | Sidebar: live children                                                   | Shipped                                              | unit       | no          | 2026-10-30 pass                        |
| SH-10 | Sidebar: keyboard and remembered branches                                | Shipped                                              | unit       | no          |                                        |
| SH-11 | Sidebar badges and dots                                                  | Shipped                                              | unit       | no          |                                        |
| SH-12 | Navigation history and view validation                                   | Shipped                                              | unit       | no          |                                        |
| SH-13 | Leave guard ("Discard changes?")                                         | Shipped                                              | unit       | no          |                                        |
| SH-14 | Opening views from the companion window                                  | Duplicate of AI-18                                   | —          | no          |                                        |
| SH-15 | Home page                                                                | Shipped                                              | unit       | no          |                                        |
| SH-16 | Home on a fresh install (Get started)                                    | Shipped                                              | unit       | no          |                                        |
| SH-17 | Backup nudge                                                             | Duplicate of DT-32                                   | —          | no          |                                        |
| SH-18 | Recent results: rate and retry                                           | Shipped                                              | unit       | no          |                                        |
| SH-19 | All activity                                                             | Shipped                                              | unit       | no          |                                        |
| SH-20 | "No model connected" notice                                              | Shipped                                              | unit       | no          |                                        |
| SH-21 | Command palette: open, search, open results                              | Shipped                                              | api        | no          |                                        |
| SH-22 | Palette: empty state, recents, and "Ask Nandi"                           | Shipped                                              | unit       | no          |                                        |
| SH-23 | Palette command mode (`>`) and local commands                            | Shipped                                              | unit       | no          |                                        |
| SH-24 | Palette server actions                                                   | Shipped                                              | api        | no          |                                        |
| SH-25 | Global keyboard shortcuts                                                | Shipped                                              | unit       | no          |                                        |
| SH-26 | Native application menu                                                  | Partial (visibility on Windows and Linux unverified) | manual     | no          |                                        |
| SH-27 | Context menu guard                                                       | Shipped                                              | unit       | no          |                                        |
| SH-28 | Toasts and native notifications                                          | Shipped                                              | manual     | no          |                                        |
| SH-29 | In-app updates                                                           | Duplicate of PL-08                                   | —          | no          |                                        |
| SH-30 | Error boundary and crash file                                            | Shipped                                              | unit       | no          |                                        |
| SH-31 | Emergency data files (recovery)                                          | Shipped                                              | api        | no          |                                        |
| SH-32 | Logging                                                                  | Partial (log file on macOS only)                     | manual     | no          |                                        |
| SH-33 | Mascot name from the CLI                                                 | Shipped                                              | unit       | no          |                                        |
| SH-34 | Mascot hidden phrase                                                     | Shipped                                              | unit       | no          |                                        |
| ST-01 | Settings page and section rail                                           | Shipped                                              | unit       | no          |                                        |
| ST-02 | Settings deep links and focus highlight                                  | Shipped                                              | unit       | no          |                                        |
| ST-03 | Models: default model                                                    | Duplicate of MD-04, MD-05, MD-06                     | —          | no          |                                        |
| ST-04 | Models: free tokens (trial)                                              | Duplicate of MD-13, MD-14                            | —          | no          |                                        |
| ST-05 | Models: provider keys (add, test, remove)                                | Duplicate of MD-01, MD-02, MD-03                     | —          | no          |                                        |
| ST-06 | Models: Claude sign-in instead of an Anthropic key                       | Duplicate of MD-11                                   | —          | no          |                                        |
| ST-07 | Models: Ollama row                                                       | Duplicate of MD-12                                   | —          | no          |                                        |
| ST-08 | Models: build the sandbox images                                         | Duplicate of RT-01, RT-02                            | —          | no          |                                        |
| ST-09 | Models: coding agent runtimes (cards)                                    | Duplicate of RT-08, RT-09, RT-10, RT-11, RT-12       | —          | no          |                                        |
| ST-10 | Models: Bedrock models and gateway                                       | Duplicate of MD-20, MD-21                            | —          | no          |                                        |
| ST-11 | Models › Advanced: enterprise config (`.nandi/llm.yaml`)                 | Duplicate of MD-18                                   | —          | no          |                                        |
| ST-12 | Models › Advanced: harness sessions                                      | Duplicate of RT-21                                   | —          | no          |                                        |
| ST-13 | Account: edit name and handle                                            | Shipped                                              | unit       | no          |                                        |
| ST-14 | Account › Details: device ID                                             | Shipped                                              | unit       | no          |                                        |
| ST-32 | Account: sign in with GitHub (one token for every github.com repository) | Shipped                                              | unit       | no          | 2026-10-30 pass (sign-out not checked) |
| ST-15 | Cloud credentials: AWS sign-in (profiles and SSO)                        | Duplicate of MD-19                                   | —          | no          |                                        |
| ST-16 | Cloud credentials: AWS access key for agents                             | Duplicate of SB-08                                   | —          | no          |                                        |
| ST-17 | Internet access                                                          | Duplicate of SB-10                                   | —          | no          |                                        |
| ST-18 | Backup: set up the git remote                                            | Duplicate of DT-18                                   | —          | no          |                                        |
| ST-19 | Backup: status, push, pull, change, remove                               | Duplicate of DT-20, DT-22, DT-25, DT-28              | —          | no          |                                        |
| ST-20 | Backup: push conflicts and merge conflicts                               | Duplicate of DT-24, DT-26                            | —          | no          |                                        |
| ST-21 | Backup: remote branches                                                  | Duplicate of DT-27                                   | —          | no          |                                        |
| ST-22 | Backup: move off the old `nandi/` branch                                 | Removed (2026-10-22, no legacy support)              | —          | no          |                                        |
| ST-23 | Backup › This computer's copy                                            | Duplicate of DT-31                                   | —          | no          |                                        |
| ST-24 | Sources (registries)                                                     | Shipped                                              | unit       | no          |                                        |
| ST-25 | Logs                                                                     | Shipped                                              | unit       | no          |                                        |
| ST-26 | Language and right-to-left layout                                        | Shipped                                              | unit       | no          |                                        |
| ST-27 | Light, dark and system mode                                              | Shipped                                              | unit       | no          |                                        |
| ST-28 | Color palettes and window icon                                           | Shipped                                              | unit       | no          |                                        |
| ST-29 | Rename the mascot                                                        | Shipped                                              | api        | no          |                                        |
| ST-30 | Appearance › Companion settings                                          | Duplicate of AI-17                                   | —          | no          |                                        |
| ST-31 | Preferences persistence                                                  | Shipped                                              | api        | no          |                                        |
| SH-35 | Home: connect your email to-do                                           | Shipped                                              | unit       | no          |                                        |

### Agents, chat and runs

| ID    | Feature                                                                 | Status             | Layer         | Needs relay | Last verified                               |
| ----- | ----------------------------------------------------------------------- | ------------------ | ------------- | ----------- | ------------------------------------------- |
| AG-01 | Agents list page                                                        | Shipped            | api           | no          |                                             |
| AG-02 | Agents list search and filters                                          | Shipped            | ui-e2e        | no          |                                             |
| AG-03 | Agent problem line and fixes                                            | Shipped            | unit          | no          |                                             |
| AG-04 | Agent row menu                                                          | Shipped            | unit          | no          |                                             |
| AG-05 | New agent dialog (template)                                             | Shipped            | api           | no          |                                             |
| AG-06 | New agent: describe it in Ask                                           | Shipped            | ui-e2e        | no          |                                             |
| AG-07 | Agent created or changed by editing files                               | Shipped            | api           | no          |                                             |
| AG-08 | skill.md format: keys the app honors                                    | Shipped            | unit          | no          |                                             |
| AG-09 | Agent status                                                            | Shipped            | api           | no          |                                             |
| AG-10 | Agent page shell and header                                             | Shipped            | ui-e2e        | no          |                                             |
| AG-11 | Pending-approval banner on the agent page                               | Shipped            | ui-e2e        | no          |                                             |
| AG-12 | Overview tab: health card                                               | Shipped            | unit + api    | no          |                                             |
| AG-13 | Overview tab: waiting, recent work, queue, about                        | Shipped            | ui-e2e        | no          |                                             |
| AG-14 | Runs tab                                                                | Shipped            | api + ui-e2e  | no          |                                             |
| AG-15 | Settings tab: General, save, revert and leave guard                     | Shipped            | ui-e2e        | no          |                                             |
| AG-16 | Settings tab: Model and runtime                                         | Shipped            | unit + ui-e2e | no          |                                             |
| AG-27 | Settings tab: Your rules (kept across updates)                          | Shipped            | unit + manual | no          | 2026-10-30 pass (saved, kept after restart) |
| AG-28 | My notes: the notes Chief of Staff keeps                                | Shipped            | unit + ui     | no          | 2026-11-01 pass after fix                   |
| AG-30 | Chief of Staff comes with Nandi                                         | Shipped            | unit          | no          | 2026-11-01 pass                             |
| AG-31 | Chief of Staff browses websites                                         | Shipped            | unit          | no          |                                             |
| AG-32 | Run plans: plan first, live checklist, finish check                     | Shipped            | unit          | no          |                                             |
| AG-33 | Playbooks: skill files an agent reads on demand                         | Shipped            | unit          | no          |                                             |
| AG-34 | Sites and accounts: site secrets, site profiles, sign out everywhere    | Shipped            | unit          | no          |                                             |
| AG-29 | Settings tab: Internet access (allowed sites)                           | Shipped            | unit          | no          |                                             |
| AG-17 | Settings tab: Permissions panel                                         | Shipped            | unit          | no          |                                             |
| AG-18 | Settings tab: Connectors and shared folders                             | Shipped            | unit          | no          |                                             |
| AG-19 | Settings tab: Sharing (delegation settings)                             | Shipped            | unit          | no          |                                             |
| AG-20 | Settings tab: Clarification policy                                      | Shipped            | unit          | no          |                                             |
| AG-21 | Settings tab: Advanced (container resources)                            | Shipped            | unit + manual | no          |                                             |
| AG-22 | Agent memory panel                                                      | Shipped            | unit + api    | no          |                                             |
| AG-23 | Delete an agent                                                         | Shipped            | api           | no          |                                             |
| AG-24 | Container start, stop, restart                                          | Duplicate of RT-04 | —             | no          |                                             |
| AG-25 | Installed (marketplace) agents: read-only banner, fork and consent gate | Shipped            | api           | no          |                                             |
| AG-26 | `verify --agent` CLI check                                              | Duplicate of RT-24 | —             | no          |                                             |
| CH-01 | Send a message to an agent                                              | Shipped            | api           | no          |                                             |
| CH-02 | Composer Chat / Ask toggle                                              | Shipped            | ui-e2e        | no          |                                             |
| CH-03 | Attachments                                                             | Shipped            | manual        | no          |                                             |
| CH-04 | Message list                                                            | Shipped            | ui-e2e        | no          |                                             |
| CH-05 | Safe markdown rendering of agent output                                 | Shipped            | unit          | no          |                                             |
| CH-06 | Streaming and the model loop                                            | Shipped            | api + manual  | no          |                                             |
| CH-07 | Tool call card                                                          | Shipped            | unit + ui-e2e | no          |                                             |
| CH-08 | Approval request card and decisions                                     | Shipped            | api           | no          |                                             |
| CH-09 | Approval expiry                                                         | Shipped            | api           | no          |                                             |
| CH-10 | Restart behavior for runs and approvals                                 | Shipped            | api           | no          |                                             |
| CH-11 | One turn per agent and the queue strip                                  | Shipped            | api           | no          |                                             |
| CH-12 | Queue panel: reorder and remove                                         | Shipped            | api           | no          |                                             |
| CH-13 | Stop the current turn                                                   | Shipped            | api           | no          |                                             |
| CH-14 | Text-only fallback warning                                              | Shipped            | api           | no          |                                             |
| CH-15 | Turn limits (max_turns, max_cost_usd, context cap)                      | Shipped            | api           | no          |                                             |
| CH-16 | Plan mode (read_only)                                                   | Shipped            | api           | no          |                                             |
| CH-17 | Verify on stop                                                          | Shipped            | unit          | no          |                                             |
| CH-18 | Compaction of long runs                                                 | Shipped            | api           | no          |                                             |
| CH-19 | Deferred tool loading (tool_search)                                     | Shipped            | unit          | no          |                                             |
| CH-20 | Automatic agent memory                                                  | Shipped            | unit + manual | no          |                                             |
| CH-21 | CLI-runtime agents in chat (Claude Code, Codex, Gemini CLI)             | Shipped            | manual        | no          |                                             |
| CH-22 | Delegation task strip (requester)                                       | Shipped            | api           | no          |                                             |
| CH-23 | Provider state panel and reassign                                       | Shipped            | manual        | yes         |                                             |
| CH-24 | Clarification and escalation cards                                      | Shipped            | api           | no          |                                             |
| CH-25 | Pilot consent prompt                                                    | Shipped            | unit          | no          |                                             |
| CH-26 | Trust graph                                                             | Shipped            | unit          | no          |                                             |
| CH-27 | Live sync: snapshots and agent events                                   | Shipped            | unit + api    | no          |                                             |
| CH-28 | Working indicator while the agent answers                               | Shipped            | unit          | no          |                                             |
| CH-29 | Select and copy chat text                                               | Shipped            | unit          | no          |                                             |
| RN-01 | Run Detail page and header                                              | Shipped            | unit + ui-e2e | no          |                                             |
| RN-02 | Overview tab: key results                                               | Shipped            | unit          | no          |                                             |
| RN-03 | Timeline tab                                                            | Shipped            | unit          | no          |                                             |
| RN-04 | Outputs tab                                                             | Shipped            | unit          | no          |                                             |
| RN-05 | Files tab                                                               | Shipped            | api + manual  | no          |                                             |
| RN-06 | Logs tab                                                                | Shipped            | unit          | no          |                                             |
| RN-07 | Approvals tab                                                           | Shipped            | unit          | no          |                                             |
| RN-08 | Ask about this run (analysis)                                           | Shipped            | api           | no          |                                             |
| RN-09 | Stop from Run Detail                                                    | Shipped            | unit          | no          |                                             |
| RN-10 | Re-run                                                                  | Shipped            | unit          | no          |                                             |
| RN-11 | Compaction lineage in Run Detail                                        | Shipped            | unit          | no          |                                             |
| RN-12 | "Part of" Request link                                                  | Shipped            | unit          | no          |                                             |
| RN-13 | Usage tokens and cost recording                                         | Shipped            | api + manual  | no          |                                             |
| RN-14 | Global runs query API                                                   | Shipped            | api           | no          |                                             |
| RN-15 | Audit log viewer                                                        | Shipped            | unit + api    | no          |                                             |

### Runtimes, models and sandbox

| ID    | Feature                                                                         | Status                                                                                     | Layer         | Needs relay | Last verified |
| ----- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------- | ----------- | ------------- |
| RT-01 | Sandbox image status panels                                                     | Shipped                                                                                    | api           | no          |               |
| RT-02 | Build the sandbox images here (advanced, optional)                              | Shipped                                                                                    | manual        | no          |               |
| RT-03 | Missing or outdated image pulled at container start                             | Shipped                                                                                    | unit + manual | no          |               |
| RT-18 | Download the pinned sandbox image from GitHub's container registry              | Shipped (needs the `images.yml` run that publishes `nandi-harness:sandbox-6` and `nandi... | unit          | no          |               |
| RT-29 | Claude Code fetched on first use into a tools volume                            | Shipped                                                                                    | unit + live   | no          |               |
| RT-04 | Agent tool container lifecycle (start, stop, restart, reconcile)                | Shipped                                                                                    | api           | no          |               |
| RT-05 | Native container configuration (mounts, resources, no published ports)          | Shipped                                                                                    | unit + manual | no          |               |
| RT-06 | Tool server in the container (shell, read_file, write_file, list_dir)           | Shipped                                                                                    | manual        | no          |               |
| RT-07 | Built-in tool catalog and On/Off switch                                         | Shipped                                                                                    | api           | no          |               |
| RT-08 | Runtime cards and detection (Nandi, Claude Code, Codex, Gemini CLI)             | Shipped                                                                                    | api + unit    | no          |               |
| RT-09 | Sign-in status per runtime card ("Ready")                                       | Shipped                                                                                    | unit + manual | no          |               |
| RT-10 | Test connection for a runtime                                                   | Shipped                                                                                    | manual        | no          |               |
| RT-11 | Runtime Details dialog                                                          | Shipped                                                                                    | unit          | no          |               |
| RT-12 | Make a runtime the default                                                      | Partial (badge only; per-runtime model picker not built: `task-x-models-and-runtimes.md... | api           | no          |               |
| RT-13 | Choose an agent's runtime                                                       | Shipped                                                                                    | ui-e2e        | no          |               |
| RT-14 | Runtime routing for a run (native vs harness, opt-in fallback)                  | Shipped                                                                                    | unit + manual | no          |               |
| RT-15 | Harness session in the container                                                | Known issue (container turn on subscription token unverified)                              | manual        | no          |               |
| RT-16 | Host placement retired                                                          | Removed (2026-10-22, no legacy support)                                                    | —             | no          |               |
| RT-17 | Credentials handed to the harness container (company identity first, keys last) | Shipped                                                                                    | unit + manual | no          |               |
| RT-30 | ACP permission answers from `allowed-tools`                                     | Shipped                                                                                    | unit          | no          |               |
| RT-19 | Turn limit for harness runs (`max_turns`)                                       | Shipped                                                                                    | unit + manual | no          |               |
| RT-20 | Harness run steps, usage and evidence                                           | Shipped                                                                                    | api           | no          |               |
| RT-21 | Recent harness sessions list and Stop                                           | Shipped                                                                                    | api           | no          |               |
| RT-22 | Harness health monitor                                                          | Shipped                                                                                    | unit          | no          |               |
| RT-23 | Custom ACP harness (`runtime: custom:<id>`)                                     | Partial (no way to register a command)                                                     | unit          | no          |               |
| RT-24 | `verify --agent <id>` CLI check                                                 | Partial (ignores Claude Code sign-in, trial and CLI runtimes)                              | unit          | no          |               |
| RT-25 | Headless and server modes                                                       | Duplicate of PL-02, PL-03                                                                  | —             | no          |               |
| RT-26 | E2E smoke prerequisites script                                                  | Duplicate of QA-05                                                                         | —             | no          |               |
| RT-27 | Canned turn engine (test fixture)                                               | Shipped                                                                                    | unit          | no          |               |
| RT-28 | List Docker containers command                                                  | Partial (backend only)                                                                     | unit          | no          |               |
| MD-01 | Add a provider key (tested before saving)                                       | Shipped                                                                                    | api           | no          |               |
| MD-02 | Key test result classification                                                  | Shipped                                                                                    | unit          | no          |               |
| MD-03 | Remove a provider key                                                           | Shipped                                                                                    | unit          | no          |               |
| MD-04 | Default model: pick provider, type any model                                    | Shipped                                                                                    | api           | no          |               |
| MD-05 | Suggested models from the provider's own list                                   | Shipped                                                                                    | api           | no          |               |
| MD-06 | "Which model calls use, and why" line                                           | Shipped                                                                                    | unit          | no          |               |
| MD-07 | Model resolution and fallback order for every call                              | Shipped                                                                                    | unit          | no          |               |
| MD-08 | Provider clients (Anthropic, OpenAI, DeepSeek, Ollama)                          | Shipped                                                                                    | unit + manual | no          |               |
| MD-09 | LLM usage and cost log (`llm.log`)                                              | Shipped                                                                                    | unit          | no          |               |
| MD-10 | Claude models through Claude Code's sign-in (Agent SDK bridge)                  | Shipped                                                                                    | manual        | no          |               |
| MD-11 | Claude sign-in shown on the Anthropic key row                                   | Shipped                                                                                    | unit          | no          |               |
| MD-12 | Ollama (local models)                                                           | Shipped                                                                                    | manual        | no          |               |
| MD-13 | Free tokens (DeepSeek trial)                                                    | Shipped                                                                                    | unit          | no          |               |
| MD-14 | Free tokens switch and automatic turn-off                                       | Shipped                                                                                    | api           | no          |               |
| MD-15 | Trial accounting fails closed                                                   | Shipped                                                                                    | unit          | no          |               |
| MD-16 | No-model notice banner                                                          | Duplicate of SH-20                                                                         | —             | no          |               |
| MD-17 | Onboarding model step                                                           | Duplicate of ON-05, ON-06, ON-07, ON-08                                                    | —             | no          |               |
| MD-18 | Team LLM config (`.nandi/llm.yaml`) and Enterprise config panel                 | Shipped                                                                                    | api           | no          |               |
| MD-19 | AWS sign-in for Bedrock (profiles, SSO login, check)                            | Shipped                                                                                    | manual        | no          |               |
| MD-20 | Bedrock model picker                                                            | Partial (chosen model is not saved as a runtime default)                                   | api           | no          |               |
| MD-21 | LiteLLM gateway for third-party Bedrock models                                  | Shipped                                                                                    | manual        | no          |               |
| SB-01 | Container hardening (tool and coding-agent containers)                          | Shipped                                                                                    | manual        | no          |               |
| SB-02 | Ask before CLI tools act (harness approvals)                                    | Shipped                                                                                    | manual        | no          |               |
| SB-03 | Egress proxy server and per-container tokens                                    | Shipped                                                                                    | api           | no          |               |
| SB-04 | LLM proxy for containers (`POST /llm`)                                          | Shipped                                                                                    | api           | no          |               |
| SB-05 | Allow-listed HTTP fetch (`POST /fetch`)                                         | Shipped                                                                                    | api           | no          |               |
| SB-06 | HTTPS tunnel (`CONNECT` via `HTTPS_PROXY`)                                      | Shipped (was Partial until #9 `d9af777`: containers were on the default bridge network)    | api           | no          |               |
| SB-07 | AWS proxy and scoped STS sessions                                               | Shipped                                                                                    | api           | no          |               |
| SB-08 | AWS access key and region for agents' AWS calls                                 | Shipped                                                                                    | api           | no          |               |
| SB-16 | Each agent's AWS identity: its own profile, the access key, or the sign-in      | Shipped                                                                                    | unit          | no          |               |
| SB-17 | Log in and set up AWS profiles in Cloud credentials                             | Shipped                                                                                    | unit          | no          |               |
| SB-09 | Egress audit log (`egress.log`)                                                 | Shipped                                                                                    | unit          | no          |               |
| SB-10 | Internet access settings page                                                   | Shipped                                                                                    | unit + ui-e2e | no          |               |
| SB-11 | Egress environment injected into agent containers                               | Shipped                                                                                    | unit          | no          |               |
| SB-12 | Secrets in the OS keyring                                                       | Duplicate of DT-04                                                                         | —             | no          |               |
| SB-13 | Agents' internal network (`nandi-agents`)                                       | Shipped                                                                                    | manual        | no          |               |
| SB-14 | Egress relay container (`nandi-egress`)                                         | Shipped                                                                                    | manual        | no          |               |
| SB-15 | Coding agents through the egress proxy, with their model hosts for the run only | Shipped                                                                                    | unit + manual | no          |               |

### Policies

| ID    | Feature                                                     | Status                                                                                     | Layer | Needs relay | Last verified |
| ----- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ----- | ----------- | ------------- |
| PO-01 | Policy files: prose and rule policies                       | Shipped                                                                                    | unit  | no          |               |
| PO-02 | Policy validation errors                                    | Shipped                                                                                    | unit  | no          |               |
| PO-03 | Studio checks policies with the enforcing parser            | Shipped                                                                                    | api   | no          |               |
| PO-04 | Targets (`applies_to`)                                      | Partial (`*`, `agent:` and `tag:` (skill.md `tags:`) enforced; `workflow:` matches noth... | unit  | no          |               |
| PO-05 | Effective permissions: tools and approvals                  | Shipped                                                                                    | unit  | no          |               |
| PO-06 | Effective permissions: files and shared folders             | Shipped                                                                                    | unit  | no          |               |
| PO-07 | Effective permissions: network and AWS                      | Shipped                                                                                    | unit  | no          |               |
| PO-08 | Effective permissions: models                               | Shipped (Nandi's own runs: the agent's model; coding agents: the CLI's model as `claude... | unit  | no          |               |
| PO-09 | Effective permissions: connectors, pilots and `run_on_host` | Shipped                                                                                    | unit  | no          |               |
| PO-10 | Broken policy files block the agents they may cover         | Shipped                                                                                    | unit  | no          |               |
| PO-11 | Agent page: the policies over an agent                      | Shipped                                                                                    | api   | no          |               |
| PO-12 | Studio policy form and agent tags                           | Shipped                                                                                    | unit  | no          |               |
| PO-13 | Policies applied to every run                               | Shipped                                                                                    | unit  | no          |               |
| PO-14 | Decisions log: who approved, and what a policy refused      | Shipped                                                                                    | unit  | no          |               |
| PO-15 | Browser controls a policy refuses (`browser.refuse`)        | Shipped                                                                                    | unit  | no          |               |
| PO-16 | Policies a starter pack ships for its own agents            | Shipped                                                                                    | unit  | no          |               |

### Workflows, triggers, requests and Inbox

| ID    | Feature                                                                          | Status                                                                                     | Layer                       | Needs relay | Last verified   |
| ----- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------- | ----------- | --------------- |
| WF-01 | Workflow YAML format and validation                                              | Shipped                                                                                    | unit                        | no          |                 |
| WF-02 | Workflow files are the gold copy (watcher, projection, read-only pack workflows) | Shipped                                                                                    | unit                        | no          |                 |
| WF-03 | Automations page with Workflows and Triggers tabs                                | Shipped                                                                                    | unit                        | no          |                 |
| WF-04 | Workflows list                                                                   | Shipped                                                                                    | unit                        | no          |                 |
| WF-05 | New workflow from a template                                                     | Shipped                                                                                    | unit                        | no          |                 |
| WF-06 | Describe a workflow in Studio                                                    | Shipped                                                                                    | ui-e2e                      | no          |                 |
| WF-07 | Import a workflow YAML file                                                      | Shipped                                                                                    | api                         | no          |                 |
| WF-08 | Workflow header and menu                                                         | Shipped                                                                                    | unit                        | no          |                 |
| WF-09 | Overview tab                                                                     | Shipped                                                                                    | unit                        | no          |                 |
| WF-10 | Graph tab (workflow canvas)                                                      | Shipped                                                                                    | unit                        | no          |                 |
| WF-11 | Step details panel                                                               | Shipped                                                                                    | unit                        | no          |                 |
| WF-12 | Edit a step from the panel                                                       | Shipped                                                                                    | unit                        | no          |                 |
| WF-13 | Runs tab and transcript                                                          | Partial (Open and **View progress** go to the workflow-run project view `work-item`, no... | unit                        | no          |                 |
| WF-14 | Configuration tab (edit the YAML)                                                | Shipped                                                                                    | unit                        | no          |                 |
| WF-15 | Workflow context panel (right sidebar)                                           | Removed                                                                                    | unit                        | no          | 2026-10-30 pass |
| WF-16 | Standalone workflow builder view                                                 | Shipped                                                                                    | unit                        | no          |                 |
| WF-17 | Run a workflow (sequential engine)                                               | Shipped                                                                                    | unit                        | no          |                 |
| WF-18 | Run source recorded ("Started by")                                               | Shipped                                                                                    | unit                        | no          |                 |
| WF-19 | Steps that ask first (workflow approvals)                                        | Shipped                                                                                    | api                         | no          |                 |
| WF-20 | Critical steps and busy agents (queueing)                                        | Shipped                                                                                    | api                         | no          |                 |
| WF-21 | Steps that may fail (`on_failure: continue`)                                     | Shipped                                                                                    | api                         | no          |                 |
| WF-22 | Runs survive restarts                                                            | Shipped                                                                                    | api                         | no          |                 |
| WF-23 | Workflow events and live refresh                                                 | Shipped                                                                                    | api                         | no          |                 |
| WF-24 | Failed-run banner and Run again                                                  | Shipped                                                                                    | unit                        | no          |                 |
| WF-25 | Manual steps (`step_type: manual`)                                               | Shipped                                                                                    | api                         | no          |                 |
| WF-26 | Complete a manual step (reply, attachments, Mark done)                           | Shipped                                                                                    | api                         | no          |                 |
| WF-27 | Workflow run project view (pause, resume, cancel)                                | Shipped                                                                                    | unit                        | no          |                 |
| WF-28 | Workflows as capability providers (`provides:`, `{{input}}`)                     | Shipped                                                                                    | api                         | no          |                 |
| WF-30 | A step that asks a colleague                                                     | Shipped                                                                                    | unit                        | yes         | 2026-10-31 pass |
| WF-29 | Workflow HTTP and command API                                                    | Shipped                                                                                    | api                         | no          |                 |
| TR-01 | Trigger rules as files (create, update, reload)                                  | Shipped                                                                                    | api                         | no          |                 |
| TR-02 | Triggers list                                                                    | Shipped                                                                                    | unit                        | no          |                 |
| TR-03 | New Trigger dialog                                                               | Shipped                                                                                    | unit                        | no          |                 |
| TR-04 | Schedule (cron) source                                                           | Shipped                                                                                    | unit                        | no          | 2026-11-01 pass |
| TR-05 | Schedule picker with plain-words preview and next runs                           | Shipped                                                                                    | unit                        | no          |                 |
| TR-06 | File source                                                                      | Shipped                                                                                    | api                         | no          |                 |
| TR-07 | Webhook source and receiver                                                      | Shipped                                                                                    | api                         | no          |                 |
| TR-08 | Webhook/device endpoint panel                                                    | Shipped                                                                                    | unit                        | no          |                 |
| TR-09 | Rotate a trigger's secret                                                        | Shipped                                                                                    | unit                        | no          |                 |
| TR-10 | Device source and receiver                                                       | Shipped                                                                                    | api                         | no          |                 |
| TR-11 | Event source (chain off other runs)                                              | Shipped                                                                                    | api                         | no          |                 |
| TR-12 | Targets and dispatch (agent or workflow)                                         | Partial (workflow targets take no input; `docs/developer/triggers.md`)                     | api                         | no          |                 |
| TR-13 | Message (input) template                                                         | Shipped                                                                                    | unit                        | no          |                 |
| TR-14 | Priority and rate limit settings                                                 | Shipped                                                                                    | unit                        | no          |                 |
| TR-15 | Turn a trigger on or off                                                         | Shipped                                                                                    | unit                        | no          |                 |
| TR-16 | Test fire                                                                        | Shipped                                                                                    | unit                        | no          |                 |
| TR-17 | Storm guard                                                                      | Shipped                                                                                    | api                         | no          |                 |
| TR-18 | Rate limit                                                                       | Shipped                                                                                    | api                         | no          |                 |
| TR-19 | Trigger page: Settings tab                                                       | Shipped                                                                                    | unit                        | no          |                 |
| TR-20 | Firings tab (history)                                                            | Shipped                                                                                    | unit                        | no          |                 |
| TR-21 | Delete a trigger                                                                 | Shipped                                                                                    | api                         | no          |                 |
| TR-22 | Trigger secrets in the OS keyring                                                | Shipped                                                                                    | api                         | no          |                 |
| TR-23 | Trigger audit log                                                                | Shipped                                                                                    | api                         | no          |                 |
| TR-24 | Trigger events and live UI                                                       | Shipped                                                                                    | api                         | no          |                 |
| TR-25 | Trigger management HTTP API                                                      | Shipped                                                                                    | api                         | no          |                 |
| TR-26 | Trigger firings as Work items                                                    | Shipped                                                                                    | unit                        | no          |                 |
| TR-27 | Calendar source: times, days and who is responsible                              | Shipped                                                                                    | unit                        | no          | 2026-11-01 pass |
| TR-28 | Calendar backup takes over                                                       | Shipped                                                                                    | api                         | yes         | 2026-10-30 pass |
| RQ-01 | Request page header and states                                                   | Shipped                                                                                    | unit                        | no          |                 |
| RQ-02 | Steps tab (system steps, plan steps, attempts)                                   | Shipped                                                                                    | unit                        | no          |                 |
| RQ-03 | Stuck banner with blocker and fix buttons                                        | Shipped                                                                                    | unit                        | no          |                 |
| RQ-04 | Outputs, Activity and Details tabs                                               | Shipped                                                                                    | unit                        | no          |                 |
| RQ-05 | Result first and rating                                                          | Shipped                                                                                    | unit                        | no          |                 |
| RQ-06 | Stop a Request                                                                   | Shipped                                                                                    | unit                        | no          |                 |
| RQ-07 | Retry a failed outgoing request (attempt n+1)                                    | Shipped                                                                                    | unit                        | no          |                 |
| RQ-08 | "Part of" a Request from a run                                                   | Duplicate of RN-12                                                                         | —                           | no          |                 |
| RQ-09 | Live Request updates                                                             | Shipped                                                                                    | unit                        | no          |                 |
| RQ-10 | An action ask becomes a Request                                                  | Shipped                                                                                    | api                         | no          |                 |
| RQ-11 | Local Request execution, linking and restart resume                              | Shipped                                                                                    | unit                        | no          |                 |
| RQ-12 | Mark a waiting step done (local Request)                                         | Shipped                                                                                    | unit                        | no          |                 |
| RQ-13 | Do this again                                                                    | Shipped                                                                                    | unit                        | no          |                 |
| RQ-14 | Save a Request as a workflow                                                     | Shipped                                                                                    | unit                        | no          |                 |
| RQ-15 | Work items API                                                                   | Partial (`next_action.description` is still an English sentence with raw step/agent ids... | api                         | no          |                 |
| RQ-16 | Inbox on Home                                                                    | Shipped                                                                                    | unit                        | no          | 2026-11-01 pass |
| RQ-17 | Inbox count badge                                                                | Shipped                                                                                    | unit                        | no          |                 |
| RQ-18 | Tool approvals in the Inbox, with expiry                                         | Shipped                                                                                    | unit                        | no          |                 |
| RQ-19 | Workflow step approvals in the Inbox                                             | Shipped                                                                                    | unit                        | no          |                 |
| RQ-20 | Held gateway requests ("hold, don't refuse")                                     | Shipped                                                                                    | api                         | yes         |                 |
| RQ-21 | Busy gateway queues requests                                                     | Shipped                                                                                    | api                         | yes         |                 |
| RQ-22 | Typed reasons on task events                                                     | Shipped                                                                                    | unit                        | no          |                 |
| RQ-23 | Other Inbox items: clarifications, missing model, trust requests, manual steps   | Shipped                                                                                    | unit                        | no          |                 |
| RQ-24 | Failed items: Retry and Dismiss                                                  | Shipped                                                                                    | unit                        | no          |                 |
| RQ-28 | Told when a request comes back                                                   | Shipped                                                                                    | unit                        | yes         | 2026-10-31 pass |
| RQ-25 | Agent run queue (one turn per agent)                                             | Duplicate of CH-11                                                                         | —                           | no          |                 |
| RQ-26 | Watch a Request in the companion                                                 | Duplicate of AI-15                                                                         | —                           | no          |                 |
| RQ-29 | Chief of Staff acts on what arrives and shows Since you were away                | Shipped                                                                                    | unit + manual               | no          |                 |
| RQ-30 | Teammates' requests take the same intake                                         | Shipped                                                                                    | unit + integration + manual | yes         |                 |
| RQ-31 | Each intake item has a Chief of Staff thread                                     | Shipped                                                                                    | unit + manual               | no          |                 |
| RQ-32 | Auto-approve rules                                                               | Shipped                                                                                    | unit + manual               | no          |                 |
| RQ-33 | Chief of Staff does a step of its own plans, and offers Save as workflow         | Shipped                                                                                    | unit + integration + manual | no          |                 |
| RQ-27 | What was asked, word for word                                                    | Shipped                                                                                    | unit + manual               | no          | 2026-10-30 pass |

### Library, capabilities, Studio and companion

| ID    | Feature                                                                                    | Status                                                                         | Layer              | Needs relay | Last verified   |
| ----- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ | ------------------ | ----------- | --------------- |
| LB-01 | Library page: tabs, kind chips, search, Can do                                             | Shipped                                                                        | unit               | no          |                 |
| LB-02 | Library cards (one permissions line, one status pill, one action)                          | Shipped                                                                        | unit               | no          |                 |
| LB-03 | Library detail drawer                                                                      | Shipped                                                                        | unit               | no          |                 |
| LB-04 | Drawer ⋯ menu: Check for update, Fork, Uninstall, Copy ID                                  | Shipped                                                                        | ui-e2e             | no          |                 |
| LB-05 | Built-in tools On/Off                                                                      | Duplicate of RT-07                                                             | —                  | no          |                 |
| LB-06 | Shared with me tab                                                                         | Shipped (content owned by the Teams/Network area)                              | manual             | yes         |                 |
| LB-07 | Sources freshness, Refresh and Sources link (Browse)                                       | Shipped                                                                        | unit               | no          |                 |
| LB-08 | Legacy route redirects into Library                                                        | Removed (2026-10-22, no legacy support)                                        | —                  | no          |                 |
| LB-09 | Install and allow (consent before install)                                                 | Shipped                                                                        | api                | no          |                 |
| LB-10 | Marketplace agent read-only banner and first-use consent                                   | Shipped                                                                        | api                | no          |                 |
| LB-11 | Fork an installed agent                                                                    | Shipped                                                                        | api                | no          |                 |
| LB-12 | Uninstall                                                                                  | Shipped                                                                        | api                | no          |                 |
| LB-13 | Check for updates and apply update                                                         | Shipped                                                                        | api                | no          |                 |
| LB-14 | Marketplace listing, detail and search APIs                                                | Shipped                                                                        | api                | no          |                 |
| LB-15 | Registry index caching and offline behaviour                                               | Shipped                                                                        | unit               | no          |                 |
| LB-16 | Marketplace sources (Settings › Sources: agents & packs)                                   | Shipped (overlaps Settings area)                                               | unit               | no          |                 |
| LB-17 | Create my community registry                                                               | Shipped                                                                        | manual             | no          |                 |
| LB-18 | Publish an agent to a registry                                                             | Shipped (docs say "in its header"; it is in the page's ⋯ menu)                 | api                | no          |                 |
| LB-19 | Publisher keys and release signing CLI (`keygen`, `sign`)                                  | Shipped                                                                        | unit               | no          |                 |
| LB-20 | Starter packs (onboarding step and view)                                                   | Shipped (overlaps Onboarding area)                                             | unit               | no          |                 |
| LB-21 | Registry JSON schemas                                                                      | Shipped (README still says "Marketplace → Registries"; now Settings › Sources) | unit               | no          |                 |
| LB-22 | Manually cloned agent folders are picked up                                                | Shipped                                                                        | api                | no          |                 |
| LB-23 | Install a starter pack from this computer (folder or .zip)                                 | Shipped                                                                        | api + unit         | no          | 2026-11-01 pass |
| CP-01 | Connector catalog and lifecycle (install, start, stop, status)                             | Shipped                                                                        | api                | no          |                 |
| CP-02 | Connector setup: config, masked secrets, Start gating                                      | Shipped                                                                        | unit               | no          |                 |
| CP-03 | Email: connect Outlook or Gmail for the Chief of Staff                                     | Shipped (Outlook and Gmail; live sign-in not yet verified)                     | unit               | no          |                 |
| CP-28 | Email: the Chief of Staff keeps My notes from new mail                                     | Shipped (live test pending)                                                    | unit               | no          |                 |
| CP-29 | Email: reading older mail in the background                                                | Shipped (live test pending)                                                    | unit               | no          |                 |
| CP-30 | Connector packages: install from the Agent Hub with a consent dialog                       | Shipped (live test pending)                                                    | api                | no          |                 |
| CP-31 | Studio: make a connector from an MCP server and publish it                                 | Shipped (live test pending)                                                    | api                | no          |                 |
| CP-32 | Access while an agent works: Allow once, Allow for this agent, Don't allow                 | Shipped (live test pending)                                                    | api                | no          |                 |
| CP-33 | Settings › Connectors › Access and up-front grants                                         | Shipped                                                                        | unit + api         | no          |                 |
| CP-34 | Calendars and files: Outlook calendar, Google Calendar, OneDrive, Google Drive             | Shipped (live test pending)                                                    | unit               | no          |                 |
| CP-35 | The Chief of Staff keeps My notes from the calendar                                        | Shipped (live test pending)                                                    | unit               | no          |                 |
| CP-04 | Shared folders on the Filesystem connector                                                 | Shipped                                                                        | unit               | no          |                 |
| CP-05 | Per-agent connectors and folder grants                                                     | Shipped                                                                        | api                | no          |                 |
| CP-06 | Connector tool calls: prefixing, approvals, audit log                                      | Shipped                                                                        | unit               | no          |                 |
| CP-22 | Stdio connectors run in their own container                                                | Shipped                                                                        | manual             | no          |                 |
| CP-23 | GitHub connector (pull requests, files, reviews)                                           | Shipped                                                                        | unit + manual      | no          | 2026-10-30 pass |
| CP-24 | AWS connector (alarms, logs, Batch jobs, approved restart)                                 | Shipped                                                                        | unit + manual      | no          |                 |
| CP-26 | Browser pilots: a browser agents drive, only on allowed sites                              | Shipped                                                                        | unit + manual      | no          | 2026-10-30 pass |
| CP-25 | Web search on the agent's allowed sites, without a key                                     | Shipped                                                                        | unit               | no          |                 |
| CP-27 | S3 Backup connector and the Personal Backup pack                                           | Shipped (pack 0.1.0 published 2026-10-30, an official starter pack)            | unit               | no          |                 |
| CP-07 | Pilot catalog, install, start, stop                                                        | Shipped                                                                        | manual             | no          | 2026-10-30 pass |
| CP-08 | Pilot health monitor and access rule                                                       | Shipped                                                                        | unit               | no          |                 |
| CP-09 | Pilot consent: grant and revoke per agent                                                  | Shipped                                                                        | api                | no          |                 |
| CP-10 | Pilot tool approvals and audit log                                                         | Shipped                                                                        | unit               | no          |                 |
| CP-36 | Browser hand-over: the agent prepares, a person publishes                                  | Shipped                                                                        | unit + integration | no          |                 |
| CP-11 | Capabilities list (Network › Sharing › Capabilities)                                       | Shipped (overlaps Network area)                                                | unit               | no          |                 |
| CP-12 | Capability detail: overview and tabs                                                       | Shipped                                                                        | unit               | no          |                 |
| CP-13 | Create, edit and delete capability definitions                                             | Shipped                                                                        | api                | no          |                 |
| CP-14 | Capability policy form                                                                     | Shipped                                                                        | unit               | no          |                 |
| CP-15 | Transfer capability ownership                                                              | Shipped                                                                        | unit               | yes         |                 |
| CP-16 | Providers, provider breakdown and evidence                                                 | Shipped                                                                        | api                | no          |                 |
| CP-17 | Ambiguity clusters and promote to schema                                                   | Shipped                                                                        | unit               | no          |                 |
| CP-18 | Delegate to a capability from its card                                                     | Shipped (overlaps Delegation area)                                             | unit               | no          |                 |
| CP-19 | Provider selection and circuit breakers                                                    | Shipped (overlaps Delegation area)                                             | unit               | no          |                 |
| CP-20 | Capability sources (registries)                                                            | Known issue (no sync-all route; filename/ID rule inconsistent)                 | api                | no          |                 |
| CP-21 | Local capability lookups and agent cards                                                   | Shipped (presence: desktop only)                                               | api                | no          |                 |
| AI-01 | Ask page layout and conversations                                                          | Shipped                                                                        | unit               | no          |                 |
| AI-02 | Grounded Studio chat (read-only nandi_* tools and guides)                                  | Shipped                                                                        | unit               | no          | 2026-10-30 pass |
| AI-03 | Chat saves what it builds (approval card)                                                  | Shipped                                                                        | api + unit         | no          |                 |
| AI-04 | Chat starts work (nandi_start_request)                                                     | Shipped                                                                        | api                | no          |                 |
| AI-05 | Artifact editor: open, forms, raw file, validation                                         | Shipped                                                                        | unit               | no          |                 |
| AI-06 | Save, Discard, Push (commit targets)                                                       | Partial (`remoteBranchAndPr` does not open the PR; returns a warning)          | api                | no          |                 |
| AI-07 | Ask box: classify, answer, plan, edit, clarify (streaming)                                 | Shipped                                                                        | api                | no          |                 |
| AI-08 | Action plan preview and Run plan                                                           | Shipped (Request execution itself belongs to the Requests area)                | api                | no          |                 |
| AI-09 | Edit preview and Apply                                                                     | Shipped                                                                        | api                | no          |                 |
| AI-10 | Ask history                                                                                | Shipped                                                                        | api                | no          |                 |
| AI-11 | Companion window and global hotkey                                                         | Shipped                                                                        | manual             | no          |                 |
| AI-12 | The bison: animation, badge, corner placement and drag-snap                                | Shipped                                                                        | unit               | no          |                 |
| AI-13 | Companion card: Needs you, Running, Recent                                                 | Shipped                                                                        | unit               | no          |                 |
| AI-14 | Companion conversation, @Agent, Request scope and live cards                               | Shipped                                                                        | unit               | no          |                 |
| AI-15 | Watch in companion                                                                         | Shipped                                                                        | unit + manual      | no          |                 |
| AI-16 | Companion notifications                                                                    | Shipped                                                                        | unit + manual      | no          |                 |
| AI-17 | Companion settings: Nandi on the desktop, start with only the bison, closing the main w... | Shipped (overlaps Settings area)                                               | manual             | no          |                 |
| AI-18 | Companion opens views in the main window                                                   | Shipped                                                                        | manual             | no          |                 |

### Teams, network, delegation and relay

| ID    | Feature                                                                   | Status                                                                                    | Layer           | Needs relay | Last verified   |
| ----- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | --------------- | ----------- | --------------- |
| TM-01 | Local identity: display name and user slug                                | Duplicate of ST-13                                                                        | —               | no          |                 |
| TM-02 | Create a team from a registry git URL                                     | Shipped                                                                                   | api             | no          |                 |
| TM-03 | Team detail: members, shared agents, packs, last sync                     | Shipped                                                                                   | api             | no          |                 |
| TM-04 | Sync team registry                                                        | Shipped                                                                                   | api             | no          |                 |
| TM-05 | Join (re-sync) a known team                                               | Partial (no UI entry point; new teams come in through TM-02)                              | api             | no          |                 |
| TM-06 | Leave team                                                                | Shipped                                                                                   | api             | no          |                 |
| TM-07 | Share an agent with a team (push a share branch, pending merge)           | Shipped                                                                                   | api             | no          |                 |
| TM-08 | Stop sharing an agent with a team                                         | Shipped                                                                                   | api             | no          |                 |
| TM-09 | Teams as a filter on People, with the team strip                          | Shipped                                                                                   | ui-e2e          | yes         |                 |
| TM-10 | Agent menu: Share with team, Callable over the relay                      | Shipped                                                                                   | ui-e2e          | yes         |                 |
| TM-11 | Agent sharing dialog ("Share {name}")                                     | Shipped                                                                                   | ui-e2e          | yes         |                 |
| TM-12 | Shared with me (agents from teams and the relay)                          | Shipped                                                                                   | api             | yes         |                 |
| TM-13 | By capability: provider ranking, Auto and pins                            | Shipped                                                                                   | api + unit      | no          |                 |
| TM-14 | Call dialog (send one task to a shared agent)                             | Shipped                                                                                   | api             | yes         |                 |
| TM-15 | Shared by me                                                              | Shipped                                                                                   | api             | no          |                 |
| TM-16 | Trust policies per caller and agent (Allow / Ask / Deny)                  | Shipped                                                                                   | api             | yes         |                 |
| TM-17 | Who may call you: list and trust graph                                    | Shipped                                                                                   | ui-e2e          | yes         |                 |
| TM-18 | Trust requests (refused "Ask" attempts), in Sharing and in the Home Inbox | Shipped                                                                                   | api             | yes         |                 |
| TM-19 | Member trust switches: Trusted peer                                       | Shipped                                                                                   | api             | yes         |                 |
| TM-20 | Pause delegations from a member                                           | Shipped                                                                                   | api             | yes         |                 |
| TM-21 | Block a member, or one of their agents                                    | Shipped                                                                                   | api             | yes         |                 |
| NW-01 | Network page shell: tabs, status pill, "How networking works"             | Shipped                                                                                   | ui-e2e          | yes         |                 |
| NW-02 | People directory                                                          | Shipped                                                                                   | api             | yes         | 2026-11-01 pass |
| NW-03 | People empty states and invite                                            | Shipped                                                                                   | ui-e2e          | yes         |                 |
| NW-04 | "Others can't see you" banner with Make reachable                         | Shipped                                                                                   | ui-e2e          | yes         |                 |
| NW-05 | Network directory events and member diffing                               | Shipped                                                                                   | api             | yes         |                 |
| NW-06 | Member profile                                                            | Shipped                                                                                   | ui-e2e          | yes         |                 |
| NW-07 | "Can {name} receive from me?" diagnosis with a copyable message           | Shipped                                                                                   | unit            | yes         |                 |
| NW-08 | Ask {name} (a plain-language request to a member's gateway)               | Shipped                                                                                   | api             | yes         |                 |
| NW-09 | Sharing: What you offer (overlaps the gateway area)                       | Shipped                                                                                   | unit            | no          |                 |
| NW-10 | Sharing: Incoming requests                                                | Shipped                                                                                   | ui-e2e          | yes         |                 |
| NW-11 | Sharing: Capabilities tab (overlaps the capability area)                  | Duplicate of CP-11                                                                        | —               | no          |                 |
| NW-12 | Reassign a task you are serving to a teammate                             | Shipped                                                                                   | api             | yes         |                 |
| NW-14 | Availability: busy or away until, and who covers                          | Shipped                                                                                   | unit + ui-e2e   | yes         | 2026-11-01 pass |
| NW-13 | Device (instance) ID                                                      | Duplicate of ST-14                                                                        | —               | no          |                 |
| RL-01 | Connection: Join a company relay, Change                                  | Shipped                                                                                   | api + ui        | yes         | 2026-11-01 pass |
| RL-02 | Test connection                                                           | Shipped                                                                                   | api             | yes         |                 |
| RL-03 | Disconnect                                                                | Shipped                                                                                   | api             | yes         |                 |
| RL-04 | Relay URL precedence (`NANDI_RELAY_URL` > config.json > none)             | Shipped                                                                                   | api             | yes         |                 |
| RL-05 | Connection status, reconnect and Details                                  | Shipped                                                                                   | api             | yes         |                 |
| RL-06 | "Make this Nandi reachable" checklist (7 rows)                            | Shipped                                                                                   | unit            | yes         |                 |
| RL-07 | Run a relay: `nandi-relay`                                                | Shipped                                                                                   | api             | yes         |                 |
| RL-08 | Relay HTTP endpoints: `/health`, `/instances`                             | Shipped                                                                                   | api             | yes         |                 |
| RL-09 | Relay token auth (`NANDI_RELAY_SECRET`)                                   | Partial (the token flow works, but there is no end-to-end UX; relays are open by default) | api             | yes         |                 |
| RL-10 | Presence: heartbeats, eviction, capability presence                       | Shipped                                                                                   | api             | yes         |                 |
| RL-11 | Two instances on one machine (`NANDI_HOME`)                               | Shipped                                                                                   | manual          | no          |                 |
| RL-12 | Agent delegation settings                                                 | Shipped                                                                                   | api             | no          |                 |
| RL-13 | `delegate_to_agent` tool, with approval                                   | Shipped                                                                                   | api             | no          |                 |
| RL-14 | Delegate a task: ranking, fallback, timeout, failover at the relay        | Shipped                                                                                   | api             | no          | 2026-11-01 pass |
| RL-15 | Inbound admission (provider side)                                         | Shipped                                                                                   | api             | yes         |                 |
| RL-16 | Delegation thread view                                                    | Shipped                                                                                   | ui-e2e          | no          |                 |
| RL-17 | Clarifications between agents (respond, escalate, policy)                 | Shipped                                                                                   | api             | no          |                 |
| RL-18 | Delegation history and audit log                                          | Shipped                                                                                   | api             | no          |                 |
| RL-27 | Find who is responsible for a topic (`find_people`)                       | Shipped                                                                                   | unit + api      | yes         | 2026-11-01 pass |
| RL-37 | "Update Nandi or the relay" (protocol version mismatch)                   | Shipped                                                                                   | unit + api      | yes         |                 |
| RL-28 | Open requests both ways (`my_requests`)                                   | Shipped                                                                                   | unit + manual   | no          | 2026-11-01 pass |
| RL-19 | Configure the git remote, credential and test                             | Duplicate of DT-18, DT-19, DT-21                                                          | —               | no          |                 |
| RL-20 | Push, pull, conflicts, branches (main and master protected)               | Duplicate of DT-22, DT-23, DT-24, DT-25, DT-26                                            | —               | no          |                 |
| RL-21 | `.nandi/llm.yaml` shared LLM config                                       | Duplicate of MD-18                                                                        | —               | no          |                 |
| RL-22 | Web server launch modes (`--server`, `--port`, `--headless`)              | Duplicate of PL-02, PL-03                                                                 | —               | no          |                 |
| RL-23 | Web auth token                                                            | Shipped                                                                                   | api             | no          |                 |
| RL-29 | A build's own relay, offered and joined only when the user agrees         | Shipped                                                                                   | unit + api      | yes         |                 |
| RL-38 | Join a company relay with a join code, and invite a teammate              | Shipped                                                                                   | api + ui        | yes         |                 |
| RL-30 | Revoking a device on the relay (a leaver)                                 | Shipped                                                                                   | api             | yes         | 2026-10-31 pass |
| RL-31 | Relay over TLS, trusting a company's own CA                               | Shipped                                                                                   | api             | yes         | 2026-10-31 pass |
| RL-32 | Warning for an unencrypted relay on another machine                       | Shipped                                                                                   | unit            | yes         |                 |
| RL-33 | Relay limits, container image and Helm chart                              | Shipped                                                                                   | api             | yes         |                 |
| RL-35 | A relay's user limit (20 free, more with a licence)                       | Shipped                                                                                   | api             | yes         | 2026-11-01 pass |
| RL-34 | Everyone on the company's relay is trusted                                | Shipped                                                                                   | unit            | yes         | 2026-11-01 pass |
| RL-39 | Device keys, pinned by the relay                                          | Shipped                                                                                   | api             | yes         |                 |
| RL-40 | "Their key changed" on a member's profile                                 | Shipped                                                                                   | unit + api + ui | yes         |                 |
| RL-41 | The relay's task ledger (operator view)                                   | Shipped                                                                                   | api             | yes         |                 |
| RL-42 | Content sealed end to end                                                 | Shipped                                                                                   | unit + api      | yes         |                 |
| RL-43 | Store-and-forward: work and answers wait at the relay                     | Shipped                                                                                   | api             | yes         |                 |
| RL-44 | The relay's management API (operator secret)                              | Shipped                                                                                   | api             | yes         |                 |
| RL-24 | WebSocket event stream (`/ws`)                                            | Shipped                                                                                   | api             | no          |                 |
| RL-25 | Frontend served over HTTP                                                 | Shipped                                                                                   | ui-e2e          | no          |                 |
| RL-26 | HTTP API as an automation surface                                         | Shipped                                                                                   | api             | no          |                 |
| RL-36 | "Connect to your company's relay" (the typed not connected)               | Shipped                                                                                   | unit + api      | no          | 2026-11-01 pass |

### Instance gateway

| ID    | Feature                                                                        | Status  | Layer           | Needs relay | Last verified   |
| ----- | ------------------------------------------------------------------------------ | ------- | --------------- | ----------- | --------------- |
| GW-01 | NANDI.md created on first launch                                               | Shipped | unit            | no          |                 |
| GW-02 | Manifest round-trip, external edits and live reload                            | Shipped | unit            | no          |                 |
| GW-03 | Gateway config and manifest API                                                | Shipped | api             | no          |                 |
| GW-04 | Gateway card at the relay: registration, heartbeat and withdrawal              | Shipped | api             | yes         |                 |
| GW-05 | Gateway status and Make reachable (API)                                        | Shipped | api             | yes         |                 |
| GW-06 | Owner from NANDI.md on the local agent cards                                   | Shipped | api             | no          |                 |
| GW-07 | Gateway admission policy for inbound requests (order and typed reasons)        | Shipped | unit            | yes         |                 |
| GW-08 | Hourly rate limit per caller, with the `gateway://rate_limited` event          | Shipped | unit            | yes         |                 |
| GW-09 | Monthly task budget and usage per caller                                       | Shipped | unit            | yes         |                 |
| GW-10 | Caller policies: created on first contact, edited in "Who may call you" → List | Shipped | api             | yes         |                 |
| GW-11 | Busy queue: gateway slots, per-caller concurrency and the wait limit           | Shipped | api             | yes         |                 |
| GW-12 | Routing: direct matches first, then the model                                  | Shipped | unit            | yes         |                 |
| GW-13 | Approval holds: "Needs my approval" waits for the owner                        | Shipped | api             | yes         |                 |
| GW-14 | Releasing a hold from the Inbox or the Request page                            | Shipped | unit            | yes         |                 |
| GW-20 | Assigned work lasts until the person ends it (`assign-me`)                     | Shipped | unit + api + ui | yes         | 2026-11-01 pass |
| GW-15 | Clarifying an unclear request                                                  | Shipped | api             | yes         |                 |
| GW-16 | Routing to a local workflow, with step progress                                | Shipped | api             | yes         |                 |
| GW-17 | Federation: passing requests to trusted peers' gateways                        | Shipped | api             | yes         |                 |
| GW-18 | What you offer: the editor, limits and Advanced (Edit as file)                 | Shipped | unit            | no          |                 |
| GW-19 | Ask the gateway (owner's gateway chat)                                         | Shipped | api             | no          |                 |

### Data, platform and quality gates

| ID    | Feature                                                      | Status                                                                                     | Layer      | Needs relay | Last verified   |
| ----- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------ | ---------- | ----------- | --------------- |
| DT-01 | Gold-copy repo layout and first-run initialization           | Shipped                                                                                    | unit       | no          |                 |
| DT-02 | Atomic, staged writes for every definition change            | Shipped                                                                                    | unit       | no          |                 |
| DT-03 | Portable settings in `.nandi/settings.yaml`                  | Shipped                                                                                    | unit       | no          |                 |
| DT-04 | Secrets only in the OS keyring                               | Shipped                                                                                    | unit       | no          |                 |
| DT-05 | Repo format version guard                                    | Shipped                                                                                    | unit       | no          |                 |
| DT-06 | Load an entity as stored in its file                         | Shipped                                                                                    | api        | no          |                 |
| DT-07 | Save an agent definition through the repo (cross-ref Agents) | Shipped                                                                                    | unit       | no          |                 |
| DT-08 | Startup load with drift detection                            | Shipped                                                                                    | unit       | no          |                 |
| DT-09 | Rebuild the projection (command and HTTP)                    | Shipped                                                                                    | api        | no          |                 |
| DT-10 | `nandi rebuild [--force]` CLI                                | Shipped                                                                                    | unit       | no          |                 |
| DT-11 | Repo status (counts, problems, watching)                     | Shipped                                                                                    | api        | no          |                 |
| DT-12 | SQLite schema                                                | Shipped                                                                                    | unit       | no          |                 |
| DT-13 | Hand edits apply live                                        | Shipped                                                                                    | api        | no          |                 |
| DT-14 | Invalid file keeps the last good version                     | Shipped                                                                                    | api        | no          |                 |
| DT-15 | Watcher fallback and read-through reads                      | Shipped                                                                                    | unit       | no          |                 |
| DT-16 | Local git status, diff and commit                            | Partial (diff and commit exposed via command/HTTP only, no UI)                             | api        | no          |                 |
| DT-17 | Backup page and empty state                                  | Shipped                                                                                    | unit       | no          |                 |
| DT-18 | Configure the remote (URL, credential, branch prefix)        | Shipped                                                                                    | api        | no          | 2026-10-30 pass |
| DT-19 | Remote credential stored in the keychain                     | Shipped                                                                                    | unit       | no          |                 |
| DT-20 | Remote status card                                           | Shipped                                                                                    | api        | no          |                 |
| DT-21 | Test connection                                              | Shipped                                                                                    | api        | no          |                 |
| DT-22 | Back up now (push)                                           | Shipped                                                                                    | api        | no          |                 |
| DT-23 | Branch protection (never main/master)                        | Shipped                                                                                    | unit       | no          |                 |
| DT-24 | Push conflict and Overwrite                                  | Shipped                                                                                    | api        | no          |                 |
| DT-25 | Restore from remote (pull)                                   | Shipped                                                                                    | api        | no          |                 |
| DT-26 | Pull merge conflicts banner                                  | Shipped                                                                                    | api + unit | no          |                 |
| DT-27 | Remote branch browser                                        | Shipped                                                                                    | unit       | no          |                 |
| DT-28 | Change or remove the remote                                  | Shipped                                                                                    | api        | no          |                 |
| DT-29 | Legacy `nandi/` branch notice and switch                     | Removed (2026-10-22, no legacy support)                                                    | —          | no          |                 |
| DT-30 | Remote events                                                | Shipped                                                                                    | api        | no          |                 |
| DT-31 | This computer's copy panel                                   | Shipped                                                                                    | unit       | no          |                 |
| DT-32 | Dashboard backup nudge (cross-ref Dashboard)                 | Shipped                                                                                    | unit       | no          |                 |
| DT-33 | `~/.nandi` copied to `~/.nandi`                              | Removed (2026-10-22, no legacy support)                                                    | —          | no          |                 |
| DT-34 | `NANDI_HOME`                                                 | Shipped                                                                                    | unit       | no          |                 |
| DT-35 | `NANDI_*` environment variables                              | Shipped                                                                                    | unit       | no          |                 |
| DT-36 | Pre-v6 install migrated to the gold-copy layout              | Removed (2026-10-22, no legacy support)                                                    | —          | no          |                 |
| DT-37 | Legacy trigger rows: secrets to keyring, rules to files      | Removed (2026-10-22, no legacy support)                                                    | —          | no          |                 |
| DT-38 | `NANDI.md` renamed to `NANDI.md`                             | Removed (2026-10-22, no legacy support)                                                    | —          | no          |                 |
| DT-39 | Enterprise LLM config (`.nandi/llm.yaml`)                    | Duplicate of MD-18                                                                         | —          | no          |                 |
| DT-40 | Local config file `~/.nandi/config.json`                     | Shipped                                                                                    | unit       | no          |                 |
| DT-41 | Data locations and reset procedure (user guide section 12)   | Shipped                                                                                    | manual     | no          |                 |
| DT-42 | Snapshot read APIs (cross-ref Agents/Runs/Dashboard)         | Shipped                                                                                    | api        | no          |                 |
| DT-43 | Live UI: notes and work folders are watched                  | Shipped                                                                                    | unit       | no          |                 |
| DT-44 | Who changed it: "Updated just now by …"                      | Shipped                                                                                    | unit       | no          |                 |
| DT-45 | Recent changes on Home                                       | Shipped                                                                                    | unit       | no          |                 |
| DT-46 | Refresh, F5 and Ctrl+R                                       | Shipped                                                                                    | unit       | no          |                 |
| DT-47 | Re-scan when the OS drops file events                        | Shipped                                                                                    | unit       | no          |                 |
| PL-01 | CLI usage, help and argument validation                      | Shipped                                                                                    | unit       | no          |                 |
| PL-02 | `--server` and `--port`                                      | Shipped                                                                                    | unit       | no          |                 |
| PL-03 | Headless mode                                                | Shipped                                                                                    | api        | no          |                 |
| PL-04 | Windows console attach and `server.log`                      | Shipped                                                                                    | manual     | no          |                 |
| PL-05 | Other CLI commands (cross-references)                        | Shipped                                                                                    | unit       | no          |                 |
| PL-06 | Single instance and window state                             | Duplicate of SH-02, SH-03                                                                  | —          | no          |                 |
| PL-07 | Per-OS window and title bar                                  | Shipped                                                                                    | manual     | no          |                 |
| PL-08 | In-app auto-updater                                          | Shipped (2026-11-03: the endpoint is the `latest.json` the release workflow attaches to... | unit       | no          |                 |
| PL-09 | Release workflow (GitHub Actions)                            | Partial (template metadata and old download names remain)                                  | manual     | no          |                 |
| PL-10 | Release preparation script                                   | Shipped                                                                                    | manual     | no          |                 |
| PL-11 | Product and binary naming (Nandi rename)                     | Shipped                                                                                    | manual     | no          |                 |
| PL-12 | Website rebrand (external repo)                              | Shipped (external)                                                                         | manual     | no          |                 |
| QA-01 | `pnpm run check:all`                                         | Shipped                                                                                    | unit       | no          |                 |
| QA-02 | ast-grep architecture rules                                  | Shipped                                                                                    | unit       | no          |                 |
| QA-03 | knip and jscpd (periodic cleanup)                            | Shipped                                                                                    | unit       | no          |                 |
| QA-04 | TypeScript bindings generation (tauri-specta)                | Shipped                                                                                    | unit       | no          |                 |
| QA-05 | E2E smoke prerequisite script (Windows)                      | Known issue (`scripts/e2e-smoke.ps1` still expects schema v18+; with schema v2 its data... | manual     | no          |                 |
| QA-06 | Test fixtures (`test-fixtures/echo-agent`)                   | Shipped                                                                                    | unit       | no          |                 |
| QA-07 | Real-install upgrade check                                   | Removed (2026-10-22, no legacy support)                                                    | —          | no          |                 |
| QA-08 | Task and helper scripts                                      | Shipped                                                                                    | manual     | no          |                 |
| QA-09 | Offline suite (`pnpm run test:offline`)                      | Shipped                                                                                    | api        | no          |                 |
