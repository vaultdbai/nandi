# Task: Machine doctor — let the model troubleshoot and set up the user's machine

**Goal:** when something on the user's machine is in the way, they ask Nandi instead of calling someone for help. Examples: Docker doesn't start; pip or npm can't reach the company's Artifactory; Docker can't log in to the company registry; a proxy or certificate is missing. The model reads the machine's state (with the user's permission), says what's wrong, and proposes fixes, which run only after the user approves each one.

**Size:** L

## Why this is a special case

Agents run only in containers, and nothing they do runs on the host (see `agents-run-only-in-container`). Troubleshooting the host has to look at the host. The way through is the one Nandi already has for host access: a **pilot** (host-side, per-agent consent, revocable, audited; `docs/developer/pilots.md`). The doctor is a pilot with a narrow, read-first tool set. It is not general host execution.

## What it can see (read, with consent)

- **The user's choice of scope:** home folder (`~`) read-only, plus any extra folders they add. Consent is per agent, shown in the agent's Permissions, revocable, and recorded in the pilot audit log.
- **Config files**, read through a redacting reader: `~/.docker/config.json` (the `auths` values redacted), `~/.docker/daemon.json`, `pip.conf`/`pip.ini`, `.npmrc`, `.yarnrc`, `~/.aws/config`, `~/.gitconfig`, shell profiles, proxy environment variables, and certificate stores (names only).
- **Allow-listed diagnostic commands**, run on the host with fixed arguments and no shell:
  - Docker: `docker version`, `docker info`, `docker context ls`
  - Windows: `wsl --status`, `wsl -l -v`, and the Docker Desktop service status; on macOS, `launchctl` for Docker
  - Package managers: `pip config list`, `npm config list`, `git config --list --show-origin`
  - Network: `nslookup`/`curl -I` against the configured registries, and the proxy settings
  - The tail of Docker Desktop's own log
- **Redaction:** tokens, passwords, `_authToken`, `auth` values and credentials in URLs are replaced before anything reaches the model. Files known to hold credentials (`~/.aws/credentials`, CLI auth files, SSO caches) are never read.

## What it can change (only with approval)

- Each fix is a proposed action, shown on an approval card **before** it runs, with a diff for file edits. Examples:
  - write a `pip.conf` index URL
  - add the registry to `.npmrc` (the user types any token into Nandi's prompt, and it goes straight to the file or the keyring, never to the model)
  - run `docker login <registry>` with credentials the user enters
  - restart Docker Desktop
  - set `HTTP(S)_PROXY`
- Every change is backed up first (`<file>.nandi-bak`) and logged in the audit log, with an "undo" action.
- No free-form shell. New fixes are added as typed actions with tests.

## Where it shows up

- A **Doctor** agent available out of the box, using this pilot.
- Nandi's own readiness checks (Docker not running, image build failed, registry unreachable) offer **Ask the doctor**, which opens the conversation with the failure already attached.
- Studio's guides gain a "troubleshoot your machine" guide, so Ask points users to it.

## Phases

- [ ] **1. Read-only diagnosis:** the pilot with the redacting reader and the allow-listed commands; the Doctor agent; "Ask the doctor" from Docker and image errors.
- [ ] **2. Approved fixes:** typed fix actions with backups, undo and audit, plus token entry that never reaches the model.
- [ ] **3. Registry and package setup:** guided Artifactory / company registry setup for pip, npm and Docker. This feeds `task-x-v1-sandbox-images.md`: once the machine can pull, image builds work.
- [ ] Catalog entries in `docs/testing/` (CP-/SB-) for each phase.

## Open questions for the owner

- Is the home folder read-only by default when the user turns the doctor on, or off until they pick folders?
- Which fixes are in phase 2, and are any too risky to offer (for example, editing Docker's daemon settings)?
