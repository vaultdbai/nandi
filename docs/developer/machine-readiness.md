# Get Your Computer Ready (machine readiness)

Agents act only inside Docker containers. Without Docker, Nandi still works as a chat assistant, but agents get no tools. This feature checks, read-only, whether this computer is ready, and tells the user what to do, step by step, on Windows, macOS and Linux. Nothing else in Nandi waits on it.

Phase 1 (built): detection, the checklist UI, the no-admin **Start Docker** action and links to official pages. Phase 2 (built for Windows and macOS, 2026-11-18): one go-ahead that runs the installs (`wsl --update` or `wsl --install`, winget or Homebrew or Docker's `.dmg`, Start Docker), and "Start Docker when Nandi opens". Linux stays phase 1 (`pkexec` installs are planned in `docs/tasks-todo/task-x-get-ready-requirements.md`).

## Rules

- Detection is read-only and makes no network call except the image check.
- Commands run with fixed arguments, no shell, no window (`CREATE_NO_WINDOW`), a timeout (5 s, 15 s for WSL/PowerShell/systeminfo; 30 min for an install step), and are never elevated.
- Installs run only after the one go-ahead. The elevated part is the installer's own process (`winget`, `wsl`, Docker's installer), which asks the OS for admin rights (UAC, the macOS password prompt). Nandi never runs elevated and never sees a password.
- Nandi never runs as admin or root. On Linux, `sudo` commands are shown to copy, never run.
- Plain words in the UI; the core's `detail` is "what Nandi saw" (a path, a version, an error line).

## Core: `services/machine/`

| File         | What it does                                                                                                                                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mod.rs`     | Types (`MachineReport`, `MachineCheck`, `CheckId`, `CheckState`, `CheckReason`, `MachineAction`), `check(fresh)`, `start_docker()`                                                                                   |
| `probe.rs`   | Runs the probes and the registry check; caches the slow ones for 60 s                                                                                                                                                |
| `parse.rs`   | Pure parsers: UTF-16 output, `wsl --version`/`--status`, PowerShell `key=value`, systeminfo, `df -Pk`, `reg query`, `docker desktop status`, `tasklist`, Docker CLI errors, `systemctl is-active`, `/etc/os-release` |
| `assess.rs`  | Pure: what the probes saw (`Observations`) → the report. A fixture test for every state on each OS                                                                                                                   |
| `ready.rs`   | Phase 2, pure: `plan(report)` (the steps, in order), `command(step)` (program and fixed arguments per OS), `approved`, and `run` (the loop over a `Runner`, checking again after each step)                          |
| `install.rs` | Phase 2: the real `Runner` (streams output lines, downloads the `.dmg`), one plan at a time, progress on `machine://ready`, the go-ahead in the decision record                                                      |

### Checks

| `CheckId`         | Windows                                                                                                                | macOS                                                 | Linux                                                                                     |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `dockerInstalled` | Uninstall key `InstallLocation`, `Program Files`, `LOCALAPPDATA`                                                       | `/Applications/Docker.app`, `lipo -archs` vs the chip | `docker`/`dockerd` on PATH, `/opt/docker-desktop`; Podman and rootless-only are `unknown` |
| `dockerRunning`   | bollard `version()` (3 s); else `docker version`, `docker desktop status`, `tasklist`                                  | same, `pgrep`                                         | bollard; else `docker version` (socket permission), `systemctl is-active docker`          |
| `wsl`             | `wsl --version` (UTF-16LE), `wsl --status`; 2.1.5 or later                                                             | —                                                     | —                                                                                         |
| `virtualization`  | PowerShell CIM `HypervisorPresent`/`VirtualizationFirmwareEnabled`; `systeminfo` (English) as fallback                 | Apple silicon, else `sysctl kern.hv_support`          | —                                                                                         |
| `disk`            | `DriveInfo` of `%LOCALAPPDATA%` (one PowerShell call with the CIM probe)                                               | `df -Pk ~`                                            | `df -Pk /var/lib/docker`                                                                  |
| `images`          | `nandi-runtime` image present (bollard), else a manifest HEAD to ghcr.io (anonymous token, 5 s, honours `HTTPS_PROXY`) | same                                                  | same                                                                                      |

### States and reasons

`CheckState`: `ok`, `missing`, `stopped`, `needsUpdate`, `blocked`, `unknown`. `CheckReason` says why, so the UI picks its steps:

| Reason                                               | State       | When                                                                                                                  |
| ---------------------------------------------------- | ----------- | --------------------------------------------------------------------------------------------------------------------- |
| `notInstalled`                                       | missing     | No Docker / no WSL                                                                                                    |
| `wrongBuild`                                         | needsUpdate | Intel Docker Desktop on Apple silicon                                                                                 |
| `podman`, `rootless`                                 | unknown     | Linux: the `docker` command is Podman's; only a rootless socket                                                       |
| `needsDocker`                                        | missing     | Docker running, when Docker isn't installed                                                                           |
| `notRunning`                                         | stopped     | Engine doesn't answer, app closed / engine stopped / `docker.service` inactive or failed                              |
| `starting`                                           | stopped     | `docker desktop status` says starting, app open with no status command, `activating`                                  |
| `desktopFailed`                                      | blocked     | "Docker Desktop is unable to start", a 500 from the engine API, or the app says running and the engine doesn't answer |
| `socketPermission`                                   | blocked     | Linux: permission denied on `/var/run/docker.sock`                                                                    |
| `oldWsl`, `kernelUpdate`                             | needsUpdate | WSL older than 2.1.5, the inbox WSL, or a kernel update message                                                       |
| `virtualizationOff`                                  | blocked     | Firmware flag off with no hypervisor; Intel Mac without VT-x                                                          |
| `lowDisk`                                            | blocked     | Less than 15 GB free                                                                                                  |
| `downloaded`, `reachable`                            | ok          | The image is here / ghcr.io answers                                                                                   |
| `unreachable`, `certificate`, `proxyAuth`, `refused` | blocked     | No answer, TLS certificate error, 407, other HTTP status                                                              |
| `notChecked`                                         | unknown     | Nandi couldn't tell                                                                                                   |

`MachineReport.dockerReady` is `dockerRunning == ok`; `ready` is every check `ok`.

### Actions (phase 1)

`MachineAction` is tagged by `kind`: `startDocker` (Windows: launches `Docker Desktop.exe`; macOS: `open -a Docker`; Linux Docker Desktop: `systemctl --user start docker-desktop`; Linux Engine: an error, the UI shows `sudo systemctl start docker`), `openLink { url }` (official Docker, Microsoft and Arch pages, per OS, chip and distribution), `checkAgain`. The fixes for "unable to start" (`wsl --shutdown`, restart Docker Desktop) stay manual steps.

### Get your computer ready (phase 2, Windows and macOS)

`MachineReport.plan` is `ready::plan(report)`, a list of `ReadyStep`:

| Report                                                 | Plan                                                                                |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| Windows, WSL missing                                   | `wslInstall`, `installDocker` if missing, then `restart`                            |
| Windows, WSL `needsUpdate`                             | `wslUpdate`, `installDocker` if missing, `startDocker` if stopped or just installed |
| Windows, WSL ok, Docker missing                        | `wslUpdate` first (owner, 2026-11-18), `installDocker`, `startDocker`               |
| macOS, Docker missing                                  | `installDocker`, `startDocker`                                                      |
| Only Docker stopped, unable to start, or starting      | empty: the Start Docker button and the manual steps                                 |
| Linux; WSL `unknown`; the Intel build on Apple silicon | nothing for that check                                                              |

| Step            | Command (no shell)                                                                                                                                                                                                                                                                                                                |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `wslUpdate`     | `wsl --update`                                                                                                                                                                                                                                                                                                                    |
| `wslInstall`    | `wsl --install --no-distribution`                                                                                                                                                                                                                                                                                                 |
| `installDocker` | Windows: `winget install -e --id Docker.DockerDesktop --accept-package-agreements --accept-source-agreements`; no winget: hands over Docker's download page. macOS: `brew install --cask docker` (`/opt/homebrew/bin/brew` or `/usr/local/bin/brew`), else the chip's `Docker.dmg` downloaded to Downloads and opened with `open` |
| `startDocker`   | `start_docker()` (phase 1)                                                                                                                                                                                                                                                                                                        |
| `restart`       | nothing: the user restarts when ready                                                                                                                                                                                                                                                                                             |

`get_computer_ready(steps)` checks fresh, refuses unless `steps` equals the plan now (the user approved what they saw), records a `computerSetup` decision and runs in the background. After each step it checks again (`check(true)`). `wslUpdate` and `installDocker` are judged by that check, not the exit code (winget exits non-zero when Docker is already there). A failure stops the plan with `ReadyFailure { step, detail, link }`; the `.dmg`, the download page and the restart stop it with a `ReadyHandover`. `ReadyProgress` (step statuses, the last 200 output lines, the latest report) is the `machine://ready` payload and what `computer_ready_status()` returns.

**Start Docker when Nandi opens:** `settings.yaml` `start_docker_on_open` (absent until answered). At launch, `machine::start_docker_on_open` spawns a check and starts Docker only when the answer is yes and Docker is `notRunning`. Startup never waits for it.

### API

| Tauri command                     | HTTP                                     | Returns                             |
| --------------------------------- | ---------------------------------------- | ----------------------------------- |
| `check_machine(fresh: bool)`      | `POST /api/rpc/check_machine`            | `MachineReport`                     |
| `start_docker()`                  | `POST /api/rpc/start_docker`             | `()`                                |
| `get_computer_ready(steps)`       | `POST /api/rpc/get_computer_ready`       | `()`, progress on `machine://ready` |
| `computer_ready_status()`         | `POST /api/rpc/computer_ready_status`    | `ReadyProgress`                     |
| `machine_settings()`              | `POST /api/rpc/machine_settings`         | `MachineSettings`                   |
| `set_start_docker_on_open(start)` | `POST /api/rpc/set_start_docker_on_open` | `()`                                |

`fresh` re-runs the slow probes (WSL, virtualization, disk, registry); without it they come from the last minute and a check takes about 50 ms. A fresh check takes 2 to 3 s on Windows.

## UI

| Where                       | Component                                                                                                                                                   |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Settings › This computer    | `settings/sections/ThisComputerSection.tsx` (polls every 10 s, Check again)                                                                                 |
| Onboarding step 4, Computer | `onboarding/StepComputer.tsx` (skippable; Continue once Docker answers)                                                                                     |
| Home                        | `machine/MachineHomeItem.tsx` until `dockerReady`                                                                                                           |
| Agent page                  | `AgentWarningBanner` renders `machine/DockerSetupPrompt.tsx` for the core's "Tool execution unavailable" warning (the `agent://warning` event is unchanged) |

`machine/machine-model.ts` is pure: the steps for each check, reason, OS and Linux family (`stepsFor`), the why sentence (`machine.why.<checkId>`), link labels, and `isToolsUnavailable`. `MachineChecklist.tsx` renders `GetReadyCard`, then rows, steps (commands copyable and `dir="ltr"`) and `MachineActions`. Data comes from `hooks/useMachine.ts` (`['machine','report']`; `['machine','ready']`, kept live by `machine://ready`; `['machine','settings']`).

`machine/ready-model.ts` is pure: when the card asks (`goAheadPlan`: the plan installs or updates something), its title, the admin and licence lines, the card's state (`readyView`: running, failed, handed over, done, ask) and when to ask "Start Docker automatically when Nandi opens?" (`asksStartOnOpen`). `GetReadyCard.tsx` renders the card and the question; `ThisComputerSection` has the switch.

## With no model: the built-in help guide

The checklist above is rule-based; so is the help guide
(`services/help/`), for when no model can answer (no key, the trial
used up, paused or unreachable, offline). It's out of the model path:
when a model exists, the model answers.

- **Content:** `services/help/guide.json`: each topic's ID, its keywords
  in en, fr, ar and ja, an optional `weight` (the generic "where things
  are" topic weighs 0.5) and its links (`settings:<section>[:<focus>]`,
  `network:<tab>`, `agents`, `library`, `automations`, `inbox`, `home`,
  `settings`, `url:https://...`). The words live in the locales:
  `help.topics.<id>.title` and `.answer`, `help.links.<label>`.
- **Matcher:** lowercase, French accents folded; each keyword found adds
  its length to the topic's score. Latin keywords must stand as words (a
  plural `s` allowed, another script next to them is a boundary); Arabic
  and Japanese ones match anywhere. Best score wins, ties go to the
  topic listed first; up to three others come along as `related`. No
  match: every topic.
- **Where it answers:** the Ask box (`AskResponse::Help`, when
  `AskLlm::has_model` is false, or when a question's model call fails,
  with the reason), a chat sent with no model (`AskBox` calls
  `help_answer`), and the help window (`HelpGuideDialog`, opened by
  **How?** on the no-model notice and "Agents need Docker to act").
- **Adding a topic:** add it to `guide.json` (keywords in all four
  languages, at least one link), add `help.topics.<id>.*` to all four
  locales, the ID to `src/lib/help.test.ts`, and a question to
  `help/tests.rs` `common_questions_find_their_topic`.

## Tests

- Rust: `services::machine::parse::tests`, `services::machine::assess::tests` (Windows, macOS, Linux fixtures for every state, including "unable to start", WSL not installed and old WSL, socket permission denied), `probe::tests::an_unreachable_registry_says_so_quickly`, and the offline suite's `with_no_relay_the_computer_checklist_answers`.
- Live: `cargo test --lib live_machine_report -- --ignored --nocapture` prints this computer's report; `live_registry_reach` checks ghcr.io.
- Rust, phase 2: `services::machine::ready::tests` (the plan for every combination, the commands per OS, the run loop with a fake `Runner`, so `cargo test` never runs `wsl` or an installer), `parse::tests::line_splitter_*`, `machine::tests` (start on open, the setting).
- Vitest: `machine-model.test.ts`, `ready-model.test.ts`, `MachineChecklist.test.tsx` (checklist, Settings section, onboarding step, Home item, the agent prompt), `GetReadyCard.test.tsx`, `OnboardingView.test.tsx`.
- Catalog: ON-16, ST-33, ST-35, SH-37, SH-38 in `docs/testing/shell-onboarding-settings.md`.
