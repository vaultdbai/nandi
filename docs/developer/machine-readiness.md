# Get Your Computer Ready (machine readiness)

Agents act only inside Docker containers. Without Docker, Nandi still works as a chat assistant, but agents get no tools. This feature checks, read-only, whether this computer is ready, and tells the user what to do, step by step, on Windows, macOS and Linux. Nothing else in Nandi waits on it.

Phase 1 (built): detection, the checklist UI, the no-admin **Start Docker** action and links to official pages. Phase 2 (planned, `docs/tasks-todo/task-x-get-ready-requirements.md`): approved installs and fixes (winget, Homebrew, `wsl --update`, `pkexec` on Linux), each with an approval card and elevation only inside its own process.

## Rules

- Detection is read-only and makes no network call except the image check.
- Commands run with fixed arguments, no shell, no window (`CREATE_NO_WINDOW`), a timeout (5 s, 15 s for WSL/PowerShell/systeminfo), and are never elevated.
- Nandi never runs as admin or root. On Linux, `sudo` commands are shown to copy, never run.
- Plain words in the UI; the core's `detail` is "what Nandi saw" (a path, a version, an error line).

## Core: `services/machine/`

| File        | What it does                                                                                                                                                                                                         |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mod.rs`    | Types (`MachineReport`, `MachineCheck`, `CheckId`, `CheckState`, `CheckReason`, `MachineAction`), `check(fresh)`, `start_docker()`                                                                                   |
| `probe.rs`  | Runs the probes and the registry check; caches the slow ones for 60 s                                                                                                                                                |
| `parse.rs`  | Pure parsers: UTF-16 output, `wsl --version`/`--status`, PowerShell `key=value`, systeminfo, `df -Pk`, `reg query`, `docker desktop status`, `tasklist`, Docker CLI errors, `systemctl is-active`, `/etc/os-release` |
| `assess.rs` | Pure: what the probes saw (`Observations`) → the report. A fixture test for every state on each OS                                                                                                                   |

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

`MachineAction` is tagged by `kind`: `startDocker` (Windows: launches `Docker Desktop.exe`; macOS: `open -a Docker`; Linux Docker Desktop: `systemctl --user start docker-desktop`; Linux Engine: an error, the UI shows `sudo systemctl start docker`), `openLink { url }` (official Docker, Microsoft and Arch pages, per OS, chip and distribution), `checkAgain`. **Phase 2 adds kinds here** (install Docker, `wsl --update`, `wsl --shutdown`, `wsl --install`, restart Docker Desktop), each recorded in the decision record after approval.

### API

| Tauri command                | HTTP                                    | Returns         |
| ---------------------------- | --------------------------------------- | --------------- |
| `check_machine(fresh: bool)` | `GET /api/machine/readiness?fresh=true` | `MachineReport` |
| `start_docker()`             | `POST /api/machine/start-docker`        | `()`            |

`fresh` re-runs the slow probes (WSL, virtualization, disk, registry); without it they come from the last minute and a check takes about 50 ms. A fresh check takes 2 to 3 s on Windows.

## UI

| Where                       | Component                                                                                                                                                   |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Settings › This computer    | `settings/sections/ThisComputerSection.tsx` (polls every 10 s, Check again)                                                                                 |
| Onboarding step 4, Computer | `onboarding/StepComputer.tsx` (skippable; Continue once Docker answers)                                                                                     |
| Home                        | `machine/MachineHomeItem.tsx` until `dockerReady`                                                                                                           |
| Agent page                  | `AgentWarningBanner` renders `machine/DockerSetupPrompt.tsx` for the core's "Tool execution unavailable" warning (the `agent://warning` event is unchanged) |

`machine/machine-model.ts` is pure: the steps for each check, reason, OS and Linux family (`stepsFor`), the why sentence (`machine.why.<checkId>`), link labels, and `isToolsUnavailable`. `MachineChecklist.tsx` renders rows, steps (commands copyable and `dir="ltr"`) and `MachineActions`, where phase 2's install buttons go. Data comes from `hooks/useMachine.ts` (one TanStack Query key, `['machine','report']`).

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
- Vitest: `machine-model.test.ts`, `MachineChecklist.test.tsx` (checklist, Settings section, onboarding step, Home item, the agent prompt), `OnboardingView.test.tsx`.
- Catalog: ON-16, ST-33, SH-37, SH-38 in `docs/testing/shell-onboarding-settings.md`.
