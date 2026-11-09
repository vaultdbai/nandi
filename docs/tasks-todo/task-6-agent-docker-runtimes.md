# Task: Per-agent Docker runtimes (custom images, ports, repo mounts, env)

**From:** design discussion 2026-10-19 (two drafts merged and critiqued; see "Decisions")
**Size:** XL — do it as a branch (`feat/agent-docker-runtimes`), one commit per phase, never on `main`

An agent can declare its own Docker workload: a prebuilt image **or** a Dockerfile + entrypoint, ports that show up as clickable links (and that our agents can call), an optional git repo / PR checkout mounted in, and user-supplied env vars and secrets. Some workloads live long (a dev server, a database, a UI); others run one command and exit. Use cases: run and test code, review a PR in isolation, ship a tool as a Dockerfile.

## What already exists (extend it, don't duplicate it)

- `services/domain/agent.rs`: `ContainerSpec{image,memory_mb,cpu}` (:187), `AgentRuntimeKind`, `RuntimePlacement`, `RuntimeConfig` (`extra` keeps unknown keys).
- `runtime.rs`: per-agent bollard container (`nandi-runtime`), skill dir ro at `/agent/skill`, workdir rw at `/agent/workdir`, `127.0.0.1`-only random port, `cap_drop ALL`, `no-new-privileges`, limits, `host.docker.internal`.
- `services/runtimes.rs`: `AgentRuntimes::get_or_start` plus the 15s reconciler. "Running" means a container labelled `nandi.agent.id`.
- `services/harness/adapter.rs`: `Placement::Container`, `docker_args` (`run -i --rm --init`, env passed as `-e NAME`, never values on argv).
- `services/egress/`: host proxy on `127.0.0.1:11435` with an allowlist and per-agent proxy tokens. `services/secrets.rs` is the keyring. `repos/git.rs` and `marketplace/source.rs` use git2.
- `marketplace/install.rs`: file allowlist, validation, atomic write, `installed.yaml`.
- `db.rs`: schema is at **v18**, so this task's migration is **v19** (or whatever is next when it starts).
- UI: `components/agents/AgentSettingsTab.tsx`, `AgentOverviewTab.tsx`, `agent-console/ContainerControls.tsx`, `use-container-control.ts`. `tauri-plugin-opener` is already a dependency.

## Decisions (critique of the drafts, folded in)

1. **The Docker config is orthogonal to the runtime kind.** Do **not** add `AgentRuntimeKind::Docker`. Kind says _who runs the LLM loop_ (Native / Claude Code / Codex / Gemini / Custom). The container block says _what workload the agent owns_. A Claude Code agent can review a PR inside a custom image just as well as a Native agent.
2. **Sidecar model.** A custom image has no Nandi MCP tool server, so the agent's tool sandbox stays `nandi-runtime`, and the custom container is a **workload** the agent drives through scoped tools (`container_exec`, `container_logs`, `container_restart`, `list_ports`, `http_request`). For harness agents with `placement: container`, the custom image _may_ replace `nandi-harness` if it has the CLI. Validate this and fall back otherwise.
3. **Docker labels are the source of truth. SQLite is a cache.** Keep the existing pattern: label `nandi.agent.id`, `nandi.workload=1`, `nandi.mode`, `nandi.run.id`. The `agent_containers` table is rebuilt by the reconciler. Its key is the container id, **not** agent id, because job mode runs several containers per agent at once.
4. **Two lifecycle modes, plus policies, instead of four modes.**
   - `service`: long-lived. Started on first use or by the user, restart-on-crash with backoff, survives turns, stopped on user action, app quit, or `idle_timeout_min`. Draft B's "interactive" mode becomes `idle_timeout_min` plus `stop_when: view_closed`. Backend lifecycle is not coupled to the UI by default.
   - `job`: a fresh `docker run --rm` **per command** (not per turn), `timeout_seconds` (default 300), exit code and bounded logs collected. Ports exist only while it runs.
   - "Build-only" is not a mode. Build plus smoke test is the **install/verify step for every agent with `build:`**. An agent that only needs an image built is just a `job` agent.
5. **Ports bind to `127.0.0.1` only.** The default is a random host port. A fixed `host:` port is allowed, but fail clearly if it's taken. Never bind `0.0.0.0`.
6. **Addresses depend on vantage point.** The runtime-context block must give the right URL for where the caller runs. The host and the UI use `http://127.0.0.1:<hostPort>`. Other containers use the `nandi-agents` docker network alias `nandi-agent-<id>:<containerPort>`. Inside the workload itself the address is `localhost:<containerPort>`. Draft B's "inside your container: localhost" was wrong for agents whose loop runs elsewhere.
7. **Untrusted input needs consent.** Marketplace agents that declare any of these require an explicit approval dialog listing exactly what they ask for:
   - a Dockerfile build (show the Dockerfile)
   - host-path mounts
   - `env_from_host`
   - extra capabilities
   - `egress_proxy: false`
   - fixed host ports

   The approval is stored per agent and is reset when any of those fields change on update.

8. **LLM provider keys never enter containers.** This matches `llm_keys.rs`. `env_from_keyring` refuses the `llm_keys` entries, and workloads that need an LLM go through the egress `/llm` route.
9. **PR review needs isolated, reproducible checkouts.** The draft B cache, keyed by URL + ref with a pull on every use, breaks parallel reviews and reproducibility. Use per-run workspaces instead (see Phase 4). A shared read-only cache can sit underneath as an optimisation.
10. **Parsing and validation belong in `domain/`** (pure), not `services/agents.rs`. Writes go through `repos/`. Secrets go through `secret_refs` / keyring, per AGENTS.md.
11. **Do not run `pnpm tauri dev` yourself.** Ask the owner to run the manual verification.
12. **Published agents ship pushed images (owner, 2026-10-27).** Nandi's own agent runtimes are pushed to the GitHub registry (`ghcr.io/...`) by CI. Anyone else who publishes an agent must declare its runtime in the spec (`container.image`) and push that image to a registry their users can reach. We don't force a particular registry: GHCR, Docker Hub, ECR or a company's private registry all work. `build:` is for authoring and local agents. A published agent may also carry its Dockerfile so it can be read or rebuilt, but installing it pulls the declared image and doesn't build it on the user's machine.

## Spec (frontmatter)

This extends `ContainerSpec`. Legacy top-level `image:`, `memory_mb:` and `cpu:` keep working and map into it.

```yaml
container:
  mode: service # service | job
  image: ghcr.io/org/tool:1.4 # either image…
  build: # …or build (context relative to the agent dir, must stay inside it)
    context: ./docker
    dockerfile: Dockerfile
    args: { NODE_VERSION: '22' }
  entrypoint: ['/app/start.sh']
  command: []
  workdir: /workspace
  ports:
    - { container: 3000, name: 'Web UI', protocol: http, path: '/' }
    - { container: 5432, name: 'Postgres', protocol: tcp, host: 15432 } # fixed host port: needs consent
  expose_ports_in_ui: true # default true
  env: { NODE_ENV: development } # non-secret defaults
  env_required: [GITHUB_TOKEN] # user must supply before start
  env_from_keyring: [{ env: GITHUB_TOKEN, secret_ref: github.token }]
  env_from_host: [TZ, LANG] # safe allowlist only; anything else needs consent
  workspace: { accepts: [repo, pr, host], path: /workspace, mode: rw }
  volumes:
    - { host: ~/code/myproject, container: /src, readonly: true } # needs consent
  resources: { memory_mb: 2048, cpu: 2.0, pids: 512, timeout_seconds: 300 }
  network: { egress_proxy: true, allow: [api.github.com, registry.npmjs.org] }
  capabilities: [] # opt-in extras, e.g. [CHOWN, SETUID]; needs consent
  healthcheck: {
      http: '/health',
      port: 3000,
      interval_seconds: 30,
      timeout_s: 60,
    } # or { command: [...] }
  test: ['npm', 'test'] # smoke test after build/install
  idle_timeout_min: 30 # service only
  stop_when: never # never | view_closed | app_quit
```

## Phases

### Phase 1 — Domain (`domain/agent.rs` + `domain/container.rs`)

- [ ] Types: `ContainerWorkload{mode, source: Image|Build, entrypoint, command, workdir, ports, env, env_required, env_from_keyring, env_from_host, workspace, volumes, resources, network, capabilities, healthcheck, test, idle_timeout_min, stop_when}`, plus `PortMapping{container, host?, name, protocol, path?}`.
- [ ] Pure validation with unit tests. It must reject:
  - port 0, duplicate ports, or a port above 65535
  - a build context that escapes the agent dir
  - reserved env names: `NANDI_*`, `HTTP(S)_PROXY`, `NO_PROXY`, the MCP token
  - `env_from_keyring` pointing at LLM keys
  - non-allowlisted `env_from_host` without a consent flag
  - `healthcheck.port` that isn't declared
- [ ] Legacy `image/memory_mb/cpu` round-trips unchanged. Unknown keys are preserved.
- [ ] Commit: `feat(domain): container workload spec for agents`

### Phase 2 — Images (`services/workloads/images.rs`)

- [ ] `Image`: pull if missing (stream progress). `Build`: tar the context, run bollard `build_image`, tag `nandi-agent/<id>:<hash>`. The hash covers the Dockerfile, the args and the context file hashes. A matching hash means the build is skipped.
- [ ] Events: `docker://build_started|build_progress|build_completed|build_failed` (with `agentId`), logs bounded.
- [ ] GC: keep the last N tags per agent, and prune on uninstall.
- [ ] Marketplace: let Dockerfile + context files through the allowlist (size cap), and add a consent dialog before the first build of an untrusted Dockerfile.
- [ ] Published runtimes (decision 12): validation rejects a published agent with a workload but no pullable `container.image` reference. Install pulls that image and reports a clear error when the registry can't be reached or needs a sign-in (for example, "Can't pull `ghcr.io/acme/tool:1.4` — check you can reach that registry, or sign in to it with `docker login`"). Nandi's own runtime images are built and pushed to GHCR by CI.
- [ ] Commit: `feat(workloads): image pull and dockerfile build with cache`

### Phase 3 — Lifecycle (`services/workloads/lifecycle.rs`, reuses `runtime.rs` hardening)

- [ ] `service`:
  - create once, wait for health
  - reconciler-managed (restart on crash, exponential backoff, give up → `error`)
  - stop on idle, user action, app quit or `stop_when`
  - commands run via bollard exec with streamed output
- [ ] `job`: `run --rm` per command, workspace mounted, timeout enforced (kill, then remove), exit code and bounded logs recorded on the run.
- [ ] Hardening is the same as `runtime.rs`, with the declared `capabilities` added back only with consent. The network is `nandi-agents` (a user-defined bridge) plus the egress proxy.
- [ ] Ports: random `127.0.0.1` binding by default, and a clear error if a fixed port is taken. Store the actual mapping.
- [ ] Health: http probe from the host, or `command` via exec, every `interval_seconds`. Emit `docker://health_changed` on transitions.
- [ ] Startup: adopt or remove orphans by label.
- [ ] Migration **v19**: `agent_containers(container_id PK, agent_id, run_id NULL, mode, status, ports_json, started_at, health_status, last_health_at)`. It's a cache rebuilt from Docker.
- [ ] Commit: `feat(workloads): service and job lifecycle with health and ports`

### Phase 4 — Workspaces and mounts (`services/workloads/workspaces.rs`)

- [ ] Git workspace:
  - git2 clone into `~/.nandi/workspaces/<agent>/<workspace-id>/`
  - supports a branch, a commit, or a PR (`refs/pull/N/head`, and record the base for diffing)
  - one workspace per run or review
  - SSH via agent/keyring, HTTPS token from the keyring, both host-side only (a token goes into the container only on explicit opt-in)
  - optional shared bare cache under it
- [ ] Host-path mounts: resolve `~`, canonicalise, reject traversal and system dirs, read-only by default.
- [ ] Windows: translate paths for Docker Desktop. Add an optional "clone into a named volume" mode for large repos (bind mounts are slow). Leave CRLF behaviour alone (no `core.autocrlf` override).
- [ ] Events: `git://clone_started|clone_completed|clone_failed`. Settings › Clear workspaces purges them. Nothing is auto-deleted while a run references it.
- [ ] Commands: `create_workspace`, `list_workspaces`, `delete_workspace`. Run input accepts a `workspace_id`.
- [ ] Commit: `feat(workloads): git/PR workspaces and host mounts`

### Phase 5 — Env and secrets (`services/workloads/env.rs`)

- [ ] Merge order: spec `env` → user overlay (non-secret, local and uncommitted, per agent) → `env_from_keyring` → allowlisted `env_from_host`.
- [ ] A missing required or keyring value blocks the start with an actionable error. For example: "Agent needs `GITHUB_TOKEN` — add it in Agent › Settings › Environment."
- [ ] Never log values. Only names, at debug level. The UI notes that env values are visible via `docker inspect`.
- [ ] Commit: `feat(workloads): env injection from spec, overlay, keyring and host`

### Phase 6 — Agent tools and runtime context

- [ ] Tools, scoped to the agent's own workloads:
  - `container_exec(cmd, timeout)`
  - `container_logs(tail)`
  - `container_restart`
  - `list_ports`
  - `http_request(port, path, method, body)`, which only reaches that agent's mapped ports
  - `run_job(cmd)` for job mode
- [ ] When the system prompt is built (in `runtime_router` / orchestrator), inject a `<runtime_context>` block with the workload status, ports (with correct addresses per decision 6), mounts and their sources, and the env **names** only.
- [ ] Agent-to-agent: when a delegate exposes services, inject `<delegated_services>` with the delegate's ports. This is same-instance only for v1. Cross-machine access (relay tunnelling) is documented as future work.
- [ ] Commit: `feat(workloads): agent tools and runtime context for owned services`

### Phase 7 — UI

- [ ] `agent-console/AgentWorkloadPanel.tsx`, shown in `AgentOverviewTab` and the console. It shows:
  - status, mode, health and uptime
  - Start / Stop / Restart / Logs / Open terminal (service)
  - job history (job)
  - a Ports list with name, URL, an **Open** button (opener) and Copy
- [ ] `AgentSettingsTab`, Container section:
  - image vs build
  - mode
  - port rows
  - workspace and volumes
  - env editor (key / value / secret toggle with a keyring picker)
  - resources
  - network allowlist

  Validate before save: build path exists, ports don't conflict, keyring refs exist.

- [ ] Consent dialog (decision 7). Build log viewer.
- [ ] TanStack Query plus the `docker://*` and `git://*` events. Zustand selectors only. All strings go in `/locales`.
- [ ] Commit: `feat(ui): workload panel, port links and container settings`

### Phase 8 — Install smoke test, docs, tests

- [ ] After install or update: build → start → health/ports → `test` → record pass/fail in `installed.yaml` and the projection → stop if `job`. Show the result in the marketplace UI.
- [ ] Docs: `docs/developer/agent-workloads.md`, update `agent-runtime.md`, `runtime-routing.md`, `egress.md`. Userguide page.
- [ ] Tests:
  - parse and validate all spec shapes, including legacy ones
  - build cache (the second build is skipped)
  - fixed vs random ports, and the error when a port is taken
  - an env error when a keyring secret is missing
  - LLM keys refused
  - workspace per run (two PR reviews don't collide)
  - health transitions (mock Docker)
  - job timeout kill
  - orphan adoption
  - the UI port links

  Put the Docker-dependent integration tests behind a "Docker available" gate.

- [ ] `pnpm rust:bindings` and `pnpm run check:all` pass after each phase.
- [ ] Commit: `test(workloads): lifecycle, workspaces, env and ports`

## Acceptance (the owner runs these manually)

- [ ] **Service:** `nginx:alpine` with port 80 → the UI shows "Running", and the Open link serves the nginx page. It survives several turns. Stop works. Idle timeout stops it.
- [ ] **Job:** `alpine` with `echo hello` → one container per command, exits, logs captured, output returned to the agent, timeout enforced.
- [ ] **Build:** a Dockerfile that installs curl builds, is tagged and passes the smoke test. A rebuild with no changes is skipped.
- [ ] **PR review:** create a workspace from `refs/pull/N/head`. The agent runs the tests in the container and reports. Two concurrent reviews use separate checkouts.
- [ ] **Env:** a missing keyring secret gives the actionable error. Once present, it's set inside the container and never appears in logs or argv.
- [ ] **Agent access:** the agent calls its own service with `http_request` and gets a response. A delegate sees the delegator's service in `<delegated_services>`.
- [ ] **Marketplace:** an agent with a Dockerfile plus a host mount triggers the consent dialog. Declining blocks the start.
- [ ] `pnpm run check:all` passes.

## Addendum: UI-testing / browser agents (2026-10-19)

Reference agent: **PR Reviewer (UI)**. It checks out a PR, reviews the diff, runs unit and e2e tests, starts the app, and drives Chrome to test the UI while the user watches.

### Two browser placements (support both)

|                            | **Host browser pilot** (default when the user is watching)                                                                                                                     | **Container browser** (unattended / untrusted)                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| What runs                  | A real Chrome window on the user's machine, driven by a host-side MCP pilot (Playwright MCP or Chrome DevTools MCP)                                                            | Chromium inside the workload image, headed on Xvfb, plus noVNC for a live view |
| Watching                   | Native window, like Antigravity's browser. No VNC, GPU available, fast                                                                                                         | Ports panel → "Live browser" (noVNC) link, or an embedded panel                |
| Reaches the app under test | Through the published port `127.0.0.1:<hostPort>` (container-internal ports are unreachable)                                                                                   | `localhost:<port>` inside the same container                                   |
| Safety                     | Reuses pilot consent, approval and audit. **Must** use an isolated throwaway profile (never the user's cookies/sessions) and an origin allowlist limited to the app under test | Container is the boundary. No host state at all                                |
| Good for                   | Interactive reviews and debugging alongside the agent                                                                                                                          | Triggers/cron, parallel reviews, untrusted PRs, reproducible runs              |

Why not host-only: the code under test is untrusted. Unattended runs can't rely on a visible desktop. Parallel reviews would share one pilot. And every pilot call needs approval unless it is `readOnlyHint`, which would prompt on every click.

### Work items

- [ ] **Browser pilot in the catalog** (`services/pilots/catalog.rs`): Playwright MCP (any OS) and/or Chrome DevTools MCP, capability `BROWSER`. It launches with an isolated, temporary profile, one browser context per run (concurrency), and blocks navigation outside the run's allowed origins (in the pilot flags if supported, otherwise enforced in `pilots/tools.rs`).
- [ ] **Per-run approval policy for pilot browser tools**: e.g. "auto-approve navigation and clicks within the allowed origins for this run", so a UI test doesn't prompt on every click. Downloads, uploads, file pickers and new origins still prompt.
- [ ] **Agent picks placement**: `browser: { placement: host-pilot | container, allowed_origins: [...] }` in skill.md, with a run-time override ("watch on my screen"). Fall back to container when no pilot is running or the run is unattended.
- [ ] **`protocol: mcp` ports**: a workload port declared as MCP (e.g. Playwright MCP inside the container) is attached to the agent as tools through the host (host stays the only MCP client, same rule as pilots).
- [ ] **Chrome in a hardened container**: `resources.shm_mb` (default 2048 for browser images). Decide `--no-sandbox` (container is the boundary) vs shipping a Chrome seccomp profile. Document the choice.
- [ ] **Detached exec**: `container_exec(cmd, detach: true)` returns a handle with `logs(handle)` and `stop(handle)`, for dev servers.
- [ ] **Run artifacts**: a `/artifacts` mount per run. Screenshots, Playwright traces and videos show in the run view, and the host pilot's screenshots land in the same place.
- [ ] Optional: embed the noVNC live view in an app panel. A consent-gated "post review to GitHub" tool.

### Acceptance

- [ ] With the host pilot running, the PR Reviewer opens an isolated Chrome window on the user's desktop, tests `127.0.0.1:<hostPort>`, can't navigate to other origins, and doesn't prompt for each in-origin click.
- [ ] With no pilot (or on a trigger), the same agent uses the container browser, the Live browser link shows it, and screenshots/traces appear in the run view.
