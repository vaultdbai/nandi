# Task: Get your computer ready (requirements check and guided installs)

**From:** owner, 2026-11-09. "Tell users they don't meet the requirements if Docker or other dependencies are missing, and help them install them, step by step, especially desktop and Mac users."
**Serves:** trust (agents act only inside containers, so Docker is what makes acting safe), and verification (the user sees exactly what's ready).
**Size:** L. Needs relay: no.
**Related:** `task-x-machine-doctor.md` (the "Ask the doctor" step comes from it later), `task-3-self-healing-installs.md` (recovery screen, plain-words errors), `services/images` (`images.dockerNotRunning` is the only Docker message today).

## Why

Without Docker, Nandi opens and works as a chat assistant, but agents get **no tools**: no files, shell or web, no connectors (Outlook, Gmail, calendars, MCP servers), no coding agents, no browser pilot, and no tool steps in workflows. Today this shows only as a small agent warning, "Tool execution unavailable: Docker is not running" (`services/chat.rs`). Most users won't understand why agents can't act.

## Owner decisions (2026-11-09)

- **Nandi offers to install** what's missing, after the user approves each install.
- **No admin rights for Nandi itself.** Nandi installs (per-user `-setup.exe`) and runs without admin rights. It asks for admin rights **only** for an install that needs them, and only after the user approves it. For example: Docker Desktop, WSL, or turning on a Windows feature.
  - The elevated step is a separate process (the installer itself, `winget`, or a `runas` launch of the official installer). Nandi never runs elevated, never keeps elevated rights, and never asks for or stores an admin password.
  - The approval card says why admin rights are needed, before the operating system's own prompt (UAC on Windows, the password prompt on macOS).
- **Onboarding:** the check is a strongly recommended step that can be skipped, with the honest line "Without Docker, agents can only talk, not act". Until it's done, it shows again as a calm item on Home.

## What Nandi checks

| Check                               | Windows                                                      | macOS                                                        | Linux                                                                                                                                                                    |
| ----------------------------------- | ------------------------------------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Docker installed                    | registry/app path                                            | `/Applications/Docker.app`, the Apple silicon or Intel build | Docker Engine (`docker` CLI, `dockerd`) or Docker Desktop for Linux; rootless Docker and Podman are noted as `unknown`, not claimed as supported                         |
| Docker running and responding       | `docker version` / API ping                                  | same                                                         | API ping; when it fails, `systemctl is-active docker` (stopped) vs permission denied on `/var/run/docker.sock` (`blocked`: join the `docker` group, then log out and in) |
| WSL 2 installed and current         | `wsl --status`, `wsl --version`                              | —                                                            | —                                                                                                                                                                        |
| Virtualization turned on            | `systeminfo` / Hyper-V requirements                          | — (Apple silicon always; Intel: VT-x)                        | —                                                                                                                                                                        |
| Disk space (about 15 GB for images) | free space on the data drive                                 | same                                                         | same (Docker's data folder, `/var/lib/docker`)                                                                                                                           |
| Nandi's images reachable            | pull a manifest from GHCR (network, proxy, company registry) | same                                                         | same                                                                                                                                                                     |
| Distribution (to tailor the steps)  | —                                                            | —                                                            | `/etc/os-release` (`ID`, `ID_LIKE`, `VERSION_ID`): Debian/Ubuntu, Fedora/RHEL, Arch, other                                                                               |

Each check reports a state, not just ✓ or ✗: `ok`, `missing`, `stopped`, `needs_update`, `blocked` (by policy or proxy), or `unknown`, with the detail Nandi saw.

## What the user sees

- **Settings › This computer:** the checklist, a **Check again** button, and live updates (polling while the page is open, and after each fix).
- **Each item that isn't ready** gets one plain sentence saying why it matters, then numbered steps for this machine:
  - **Docker missing:**
    - _Windows:_ **Install for me** runs `winget install Docker.DockerDesktop` after approval, and Windows asks for admin rights. Or **Download**, a link to docker.com, with what to click.
    - _macOS:_ **Download** the right build for the chip, or **Install for me** through Homebrew if `brew` is present. macOS may ask for the password for Docker's helper.
  - **Docker stopped:** **Start Docker** (launches Docker Desktop, no admin), then waits and re-checks.
  - **"Docker Desktop is unable to start"** (seen on the dev machine 2026-11-09): guided fixes, in order, each run only after approval:
    1. `wsl --update`
    2. `wsl --shutdown`
    3. Restart Docker Desktop
    4. Virtualization: how to turn it on in BIOS/UEFI, with keys for common PC makers
    5. Docker's own "Reset to factory defaults", explaining what it removes
  - **Linux, Docker missing:** numbered steps for the distribution family (Debian/Ubuntu, Fedora/RHEL, Arch, other), with copyable commands and a link to Docker's official install page for that distribution.
  - **Linux, Docker stopped:** the copyable command `sudo systemctl start docker` (phase 1 never elevates inside Nandi).
  - **Linux, permission denied on the Docker socket:** `sudo usermod -aG docker $USER`, then log out and back in.
  - **WSL missing:** **Install WSL** (`wsl --install`, admin), then "Restart your computer when you're ready". Nandi remembers where it was.
  - **Images unreachable:** proxy or certificate hints, and the company-registry setup (later, from the machine doctor).
  - **Install blocked by the company:** a ready-to-send note for IT, with what to install, why, and the exact versions. A copy button.
- **Wherever a run would need tools** (an agent run, a connector, a workflow step, the browser pilot), the agent warning is replaced by a calm prompt: "Agents need Docker to act. Set up". It opens this checklist.
- **Later:** **Ask the doctor** opens the machine-doctor chat with the failed check attached.

## Linux, phase 2

Installs through the distribution's package manager run only after approval, elevated with `pkexec` (the polkit prompt), never by Nandi running as root. When `pkexec` isn't available, Nandi shows the commands to run in a terminal instead.

## Phases

- **Phase 1 (built 2026-11-09):** Steps 1 and 3, plus the no-admin **Start Docker** action and links to official download and docs pages. Windows, macOS and Linux.
- **Phase 2:** Step 2 (the approved install and fix actions, with elevation in their own process), then Step 5.

## Rules

- Detection is read-only, fast, and makes no network calls except the image check. Nothing is installed or started without approval.
- Every install or fix is a typed action with fixed arguments: no free-form shell. It is recorded in the decision record (`.nandi/decisions/`) as approved by the user.
- Elevation happens only inside the approved action's own process, and Nandi checks the result afterwards (re-runs the check).
- Plain words, no stack traces. Everything is in en/fr/ar/ja.
- Nothing else in Nandi blocks on this: the app starts and works without Docker, as it does without a relay.

## Steps

1. Detection service (`services/machine/` or similar): typed checks per OS, results as data (state, detail, suggested actions). Unit tests with fake command outputs for each state.
2. Typed actions: install Docker (winget / brew / download link), start Docker, `wsl --update`, `wsl --shutdown`, `wsl --install`, open docs links. Each with an approval card, a reason line for elevation, and a re-check after.
3. UI: Settings › This computer; the onboarding step; the Home item; "Agents need Docker to act" replacing the tool warning.
4. Docs (`docs/developer/`, user guide), catalog entries (`docs/testing/`), and the offline suite (the checklist renders with no relay).
5. Manual checks on a Windows machine without Docker, one with Docker stopped, a Mac (Apple silicon) and a Linux machine (Ubuntu, Docker stopped and socket permission denied), with screenshots.
