# Runtimes, sandbox, models and egress

What runs an agent's turns (Nandi's own loop, or a coding-agent CLI in the `nandi-harness` container), the sandbox images and containers, the LLM providers and keys, the free trial, and the egress proxy and container guard rails. Settings pages in this area: **Settings → Models & runtimes** (id `models`; `focus: runtimes` scrolls to Coding agents), **Settings → Cloud credentials** (`cloud`), **Settings → Internet access** (`internet`). Code roots: `src-tauri/src/runtime.rs`, `src-tauri/src/services/images/`, `src-tauri/src/services/{runtimes,llm_keys,llm_trial,secrets}.rs`, `src-tauri/src/services/harness/`, `src-tauri/src/services/egress/` (`relay.rs` for the agents' network), `src-tauri/src/mcp_client.rs`, `src-tauri/src/llm/`, `runtimes/`, `src/components/{harness,models,preferences}/`, `src/components/settings/sections/{ModelsSection,RuntimesSection,CloudCredentialsSection,InternetAccessSection}.tsx` (`RuntimesSection.tsx` exports `RuntimesPanel`).

## Sandbox images

### RT-01 — Sandbox image status panels

- **What:** Two panels show whether each sandbox image is here, outdated, missing (with its download size) or downloading (bytes and a progress bar), or whether Docker is not running: "Tool sandbox" (`ghcr.io/vaultdbai/nandi-runtime:sandbox-6`, Nandi's own agents' tools) and "Coding agents' sandbox" (`ghcr.io/vaultdbai/nandi-harness:sandbox-6`, Codex / Gemini CLI / Claude Code). Nothing has to be done here first: Nandi downloads an image the first time an agent needs it (RT-18). **Download now** is only for getting it ahead of time, or trying again.
- **Where:** Settings → Models & runtimes → "Nandi (built in)" group (Tool sandbox) and "Coding agents" group (Coding agents' sandbox). `src/components/harness/SandboxImagePanel.tsx` (`SandboxImagePanel`), `src/components/harness/image-progress.ts`, `src/hooks/useImages.ts`, `src-tauri/src/commands/images.rs` (`get_image_status`), `src-tauri/src/services/images/mod.rs` (`image_status`), `services/images/registry.rs` (`download_size`). HTTP `GET /api/images/{image}` (`harness` | `runtime`).
- **Preconditions:** None (Docker optional for the negative states).
- **Steps:** 1. Stop Docker Desktop, open the page. 2. Start Docker, image not on this computer. 3. Download it (RT-18). 4. Tag an image built before `org.nandi.sandbox=6` as `ghcr.io/vaultdbai/nandi-runtime:sandbox-6` (`docs/developer/sandbox-test-checklist.md` 1.1).
- **Expected:** 1. "Docker isn't running. Start Docker Desktop, and Nandi downloads what agents need." and the button disabled. 2. "Not on this computer yet. Nandi downloads it (<size>) the first time an agent needs it." with **Download now (<size>)** (no size when the registry can't be reached: "Download now"). No build button anywhere in the panel. 3. "Ready (ghcr.io/vaultdbai/nandi-runtime:sandbox-6)." and no button (a local build adds "built here by Nandi <version>"). 4. "From an older Nandi (...). Nandi downloads the current one the next time an agent needs it." and **Download now**.
- **Edge / negative cases:** Docker socket unreachable vs ping failing; page opened mid-download shows that state, its progress and log, including a download a run started (RT-18).
- **Automation:** Covered by: `src/components/harness/SandboxImagePanel.test.tsx`, `image-progress.test.ts`, Rust tests in `src-tauri/src/services/images/` (`mod.rs`, `registry.rs`) | Layer: api (`GET /api/images/{image}`) + unit | Gap: no test with a real Docker daemon.
- **Needs relay:** no
- **Status:** Shipped

### RT-02 — Build the sandbox images here (advanced, optional)

- **What:** An advanced option nothing requires: build a sandbox image on this computer from the build contexts compiled into the app (`runtimes/nandi-harness`, `runtimes/nandi-runtime`) instead of downloading it. Same name and labels as the published image. Streams output live into the image's panel (RT-01), keeps the last 300 lines, and refuses a second job of the same image while one runs.
- **Where:** Settings → Models & runtimes → Advanced → "Build the sandboxes on this computer (optional)" → **Build Tool sandbox here** / **Build Coding agents' sandbox here**. `src/components/harness/SandboxImagePanel.tsx` (`SandboxBuildPanel`), `src-tauri/src/commands/images.rs` (`build_image`), `src-tauri/src/services/images/mod.rs` (`start_build`, `context_tar`, `proxy_env`). HTTP `POST /api/images/{image}/build`. Event `images://pull` with `build: true`.
- **Preconditions:** Docker running; network access to Docker Hub / npm / PyPI (or a corporate proxy in `HTTP(S)_PROXY`).
- **Steps:** 1. Open Advanced, click **Build Coding agents' sandbox here**. 2. Leave the page and come back. 3. Start the same build again while it runs (via API). 4. Wait for the end.
- **Expected:** The button shows "Building…"; the image's panel shows "Building. This takes a few minutes the first time; you can leave this page." and its output opens. No toast (builds report in Settings only). A second job returns "ghcr.io/vaultdbai/nandi-harness:sandbox-6 is already being downloaded or built". On success the panel shows "Ready (..., built here by Nandi x.y.z)"; `docker image inspect` shows `org.nandi.sandbox=6` and `org.nandi.built-by`. The harness image built here has no Claude Code either (RT-29).
- **Edge / negative cases:** build failure shows "The last try didn't work: <error>"; `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` in the app's environment are passed as build args; no cancel button. Onboarding, runs and the runtime cards never ask for a build.
- **Automation:** Covered by: `SandboxImagePanel.test.tsx` (`SandboxBuildPanel`), `services/images/mod.rs` tests (context tar covers every COPY, no Anthropic package in the harness Dockerfile, log ring) | Layer: manual (needs Docker and internet) | Gap: no integration test that runs a build.
- **Needs relay:** no
- **Status:** Shipped

### RT-03 — Missing or outdated image pulled at container start

- **What:** Every container start goes through one image service, `images::ensure`: the image must be here with the label `org.nandi.sandbox` at `runtime::SANDBOX_VERSION` (6: the harness image without Claude Code and with the tools volume). Missing or older, it is downloaded (RT-18) and the label checked again; the run waits. Custom `image:` values must inherit the label (`FROM nandi-runtime`) and are never pulled.
- **Where:** `src-tauri/src/services/images/mod.rs` (`ensure`), `src-tauri/src/runtime.rs` (`start_container`, `image_is_current`), `services/orchestrator/runtime_router.rs` (`sandbox_ready`), `services/connectors/mcp.rs` (`spawn_in_container`), `services/harness/mod.rs` (`test_config`).
- **Preconditions:** Docker running; an agent that uses `shell`/file tools (native) or a CLI runtime.
- **Steps:** 1. `docker rmi ghcr.io/vaultdbai/nandi-runtime:sandbox-6`, run a native agent that calls `shell`. 2. Tag an image without the label as `ghcr.io/vaultdbai/nandi-runtime:sandbox-6`, run again. 3. Set `image: foo:latest` (not present) in skill.md. 4. Offline, remove the harness image and run a Codex agent.
- **Expected:** 1. The run waits while Nandi downloads the image (a toast and Settings show the progress), then runs. 2. Nandi downloads the published image over it, then runs. 3. "Container image foo:latest not found locally". 4. The run fails with "Couldn't download ghcr.io/vaultdbai/nandi-harness:sandbox-6 (...). Check that this computer is online and can reach ghcr.io, then try again." and the toast offers **Try again**. Never a hint to build.
- **Edge / negative cases:** a CLI agent with `runtime_config.fallback: native` runs natively when the download fails; a downloaded image whose label is still older says "Downloaded ..., but it isn't built for this Nandi (sandbox 6)".
- **Automation:** Covered by: `runtime.rs` tests (`image_is_current`), `runtime_router.rs` (`an_unavailable_harness_falls_back_only_when_asked`), live `first_use_claude_code_live`, `codex_runs_with_no_extra_step_live` (ignored) | Layer: unit + manual | Gap: the pull path isn't tested without Docker.
- **Needs relay:** no
- **Status:** Shipped

### RT-18 — Download the pinned sandbox image from GitHub's container registry

- **What:** Both of Nandi's sandbox images are published to `ghcr.io/vaultdbai/nandi-runtime` and `ghcr.io/vaultdbai/nandi-harness` by `.github/workflows/images.yml`. The app pulls the tag pinned to its sandbox version (`sandbox-6`, from `runtime::SANDBOX_VERSION`), never `latest`. A run, a connector or the harness Test button that needs a missing image starts the download by itself and waits; **Download now** starts it ahead of time. Its size comes from the registry before the pull; progress is bytes of that size.
- **Where:** Settings → Models & runtimes → Download now; a toast whichever view is open (`src/components/harness/SandboxProgressToasts.tsx`, mounted in `App.tsx`). `runtime.rs` (`DEFAULT_RUNTIME_IMAGE`, `HARNESS_IMAGE`: the one place the names live), `services/images/mod.rs` (`ensure`, `start_download`, `download`, `LayerBytes`, `pull_line`), `services/images/registry.rs` (`download_size`), `commands/images.rs` (`pull_image`), HTTP `POST /api/images/{image}/pull`, event `images://pull` (`{image, build, line, progress: {doneBytes, totalBytes}, finished, error}`). Workflow tags: `sandbox-<N>` on every publish, `main` on pushes to main touching `runtimes/`, `<X.Y.Z>` and `latest` on a `v*` tag.
- **Preconditions:** Docker running; the images published and public (or a registry mirror). For the negative case, no network.
- **Steps:** 1. `docker rmi` both images, open the page, click **Download now (<size>)** on "Tool sandbox". 2. Leave and come back. 3. Remove it again and run a native agent that calls `shell` from the agent page. 4. Remove the harness image and run a Codex agent. 5. Offline, click Download now.
- **Expected:** 1. "Downloading…", a progress bar, "Downloading: <done> of <total> (<n>%). You can leave this page.", the output shows one line per layer change (`<layer>: Pull complete`, no byte counters), then "Ready (ghcr.io/vaultdbai/nandi-runtime:sandbox-6)." 2. Same state, progress and log. 3. A toast "Downloading: Tool sandbox (<n>% of <total>)…" updates in place and ends "Ready: Tool sandbox"; the run then runs. 4. Same with "Coding agents' sandbox". 5. "The last try didn't work: Couldn't download ... Check that this computer is online and can reach ghcr.io, then try again." and **Try again**; a run that needed it shows the toast "Couldn't download: ..." with **Try again**.
- **Edge / negative cases:** a download while a build runs (or the reverse) is refused; `docker image inspect` of a pulled image shows `org.nandi.sandbox=6`, `org.opencontainers.image.vendor=VaultDB.ai` and the licence label; a registry that wants a login gives no size (button "Download now"), the pull goes through the host's Docker login.
- **Automation:** Covered by: `services/images/mod.rs` tests (`the_images_are_pinned_to_this_app_s_sandbox_version`, `pull_lines_leave_the_byte_counters_to_the_progress`, `progress_sums_the_layers_against_the_registry_size`, `a_failed_download_says_to_try_again_never_to_build`), `registry.rs` tests (reference, challenge, platform, sizes), `SandboxImagePanel.test.tsx`, `SandboxProgressToasts.test.tsx` | Layer: unit | Gap: no test pulls from a registry (no network in tests).
- **Needs relay:** no
- **Status:** Shipped (needs the `images.yml` run that publishes `nandi-harness:sandbox-6` and `nandi-runtime:sandbox-6` before a fresh install can download)

### RT-29 — Claude Code fetched on first use into a tools volume

- **What:** The harness image has no Claude Code: Anthropic's licence doesn't clearly allow redistributing it (nor the Claude Agent SDK its ACP adapter needs). The first time a `runtime: claude-code` agent runs (or Test connection, or **Get it now**), Nandi runs a one-off harness container that installs the adapter and SDK with `npm ci` from the lockfile shipped in the image (integrity-checked) into the Docker volume `nandi-tools-claude-code-<hash>`. Later runs mount it read-only at `/opt/nandi-tools/claude-code` with no wait. A new lockfile in a Nandi update uses a new volume and removes the old one. The user accepts Anthropic's terms by signing in to Claude Code.
- **Where:** Settings → Models & runtimes → Coding agents → "Claude Code" panel (`src/components/harness/ClaudeCodeToolsPanel.tsx`); the toast "Getting Claude Code (one time, about <size>)…" (`SandboxProgressToasts.tsx`). `services/images/tools.rs` (`ToolBundle`, `ensure`, `start_install`, `status`, `install_args`, `marker_args`, `remove_old_volumes`), `runtimes/nandi-harness/install-tools.sh`, `runtimes/nandi-harness/claude-code/package-lock.json`, `commands/images.rs` (`get_tools_status`, `install_tools`), HTTP `GET /api/tool-bundles/claudeCode`, `POST /api/tool-bundles/claudeCode/install`, event `images://tools` (`{bundle, downloadBytes, line, finished, error}`).
- **Preconditions:** Docker running; the harness image (pulled by itself); npm reachable (`registry.npmjs.org`, or the proxy in `HTTP(S)_PROXY`).
- **Steps:** 1. `docker volume rm` every `nandi-tools-claude-code-*` volume, open the page. 2. Run a Claude Code agent. 3. Quit and restart Nandi, run it again. 4. `docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-6 sh -c 'ls /usr/local/lib/node_modules'`. 5. Offline, remove the volume and click **Get it now**.
- **Expected:** 1. "Fetched the first time an agent uses it (one time, about 237 MB)." and **Get it now**. 2. The toast "Getting Claude Code (one time, about 237 MB)…", the panel's output shows npm's lines, then "Claude Code is ready." and the run starts (it then needs the Claude sign-in as before, RT-09). `docker volume ls` shows one `nandi-tools-claude-code-<hash>`. 3. No toast, no install: the run starts at once (the check takes about a second). 4. `@google`, `@openai`, `@zed-industries`, `corepack`, `npm`: no `@anthropic-ai` and no `@agentclientprotocol`. 5. "The last try didn't work: Couldn't get Claude Code (...). Check that this computer is online and can reach registry.npmjs.org, then try again." and **Try again**.
- **Edge / negative cases:** a second install while one runs waits for it (a run) or is refused (the button); a half-finished volume (app killed mid-install) fails the check and is installed again; Codex and Gemini CLI never install anything; the run's container mounts the volume `:ro` and keeps `--read-only`.
- **Automation:** Covered by: `services/images/tools.rs` tests (volume named by the lockfile, lockfile pins `ACP_VERSION` with integrity, hardened install args, read-only marker check), `adapter.rs` (`docker_run_names_variables_but_never_their_values`: the `:ro` mount), `ClaudeCodeToolsPanel.test.tsx`, `SandboxProgressToasts.test.tsx`; live `runtime_router::tests::first_use_claude_code_live` (ignored: Docker and npm) | Layer: unit + live | Gap: the live test isn't in CI.
- **Needs relay:** no
- **Status:** Shipped

## Native agent tool containers

### RT-04 — Agent tool container lifecycle (start, stop, restart, reconcile)

- **What:** Each native agent gets its own `nandi-runtime` container, started on first tool use or from the agent page, stopped/restarted from the agent page's menu; a reconciler notices containers stopped outside Nandi every 15 s.
- **Where:** Agent page → "⋯" menu → Start container / Stop container / Restart container (restart asks "Restart the container?": "The current turn will stop."). `src/components/views/AgentView.tsx`, `src/lib/agent-actions.ts` (`controlContainer`), `src-tauri/src/commands/agents.rs` (`start_agent`, `stop_agent`, `restart_agent`), `src-tauri/src/services/runtimes.rs` (`AgentRuntimes::get_or_start`, `restart`, `stop`, `reconcile`), `src-tauri/src/runtime.rs` (`start_container`, `stop_container`, `container_running`). Event `agent://agents_changed`; warning banner after `agent://warning` (`src/components/agent-console/ContainerControls.tsx`).
- **Preconditions:** Docker running, `ghcr.io/vaultdbai/nandi-runtime:sandbox-6` built.
- **Steps:** 1. Start the agent's container. 2. `docker ps` . 3. `docker stop nandi-agent-<tag>-<id>` from a terminal, wait 15 s. 4. Restart from the menu.
- **Expected:** Container named `nandi-agent-<tag>-<id>` (`runtime::container_name`; `<tag>` is `nandi_config::instance_tag`, eight hex characters from this Nandi's folder) is running with labels carrying the agent id and the instance tag; the agent overview shows the state; after the external stop the state returns to Idle; Restart replaces the container (proxy token re-issued) and opens a new tool-server session over `docker exec` (RT-06). A failed start shows "Container: couldn't start" on the agent page menu and the Overview health card, and an error toast.
- **Edge / negative cases:** Two Nandis on one machine (separate `NANDI_HOME`s) running the same agent: each gets its own container (`nandi-agent-`, `nandi-harness-` and `nandi-connector-` names all carry the tag), starting one doesn't remove the other, and each agent page shows only its own container as running (filter on label `nandi.instance`); Docker stopped mid-session; agent that only chats needs no container; the agents' network or the egress relay can't be set up (SB-13, SB-14): nothing starts; tool server doesn't answer within 30 s ("The agent's tool server did not answer").
- **Automation:** Covered by: `services/runtimes.rs` tests, `runtime.rs` tests | Layer: api (`start_agent` etc. via HTTP agent routes) + manual (Docker) | Gap: no Docker-backed integration test.
- **Needs relay:** no
- **Status:** Shipped

### RT-05 — Native container configuration (mounts, resources, no published ports)

- **What:** The tool container mounts the agent folder read-only at `/agent/skill`, a per-agent workdir `~/.nandi/local/work/<agent>` at `/agent/workdir`, and its shared folders at `/agent/folders/<id>` (read-only unless granted write); memory default 1024 MB, 1 CPU (overridable in skill.md). skill.md `shell_timeout_s` reaches the container as `NANDI_SHELL_TIMEOUT_SECONDS` (absent when unset). Since #9 it joins only the internal `nandi-agents` network (SB-13), publishes no ports, has no `host.docker.internal` mapping, and runs with `init` so the shell tool's leftovers are reaped.
- **Where:** `src-tauri/src/runtime.rs` (`container_config`, `folder_mounts`, `memory_limit_bytes`, `nano_cpus`; `relay::ensure_ready` before create).
- **Preconditions:** Docker, `nandi-runtime` built, an agent with tools (and optionally a shared folder).
- **Steps:** 1. Start the agent. 2. `docker inspect` the container; `docker port nandi-agent-<tag>-<id>` (`docs/developer/sandbox-test-checklist.md` 4.5). 3. From a tool call, write to `/agent/skill` and to a read-only shared folder.
- **Expected:** Mounts as above; `HostConfig.NetworkMode` = `nandi-agents`, `Init: true`, no `PortBindings`, no `ExposedPorts`, no `ExtraHosts`; `docker port` prints nothing; env `NANDI_AGENT_ID` plus the egress variables (SB-11), no MCP token and no `NANDI_*` names; writes to read-only mounts fail.
- **Edge / negative cases:** skill.md `memory_mb` / `cpu` overrides; workdir is created if missing; only the relay (SB-14) carries the `host.docker.internal:host-gateway` mapping.
- **Automation:** Covered by: `runtime.rs` tests (folder mounts; the config test asserts `network_mode` `nandi-agents`, no port bindings or extra hosts, no `MCP_TOKEN` or `API_KEY` in env); `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images) `the_native_container_is_locked_down` inspects the started container: exactly the agent folder (read-only), the workdir (writable) and a read-only shared folder mounted, 1024 MB, 1 CPU, `nandi-agents` only, `Init`, no port bindings, exposed ports or extra hosts, env `NANDI_AGENT_ID` plus the proxy variables and no key; writes to the read-only mounts fail | Layer: api (Docker) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RT-06 — Tool server in the container (shell, read_file, write_file, list_dir)

- **What:** `nandi-runtime` is a Python MCP server exposing `shell` (30 s timeout, or skill.md `shell_timeout_s` up to 3600 s), `read_file`, `write_file`, `list_dir`, all confined to `/agent/workdir`. Since #9 it speaks MCP over stdio: the host runs `docker exec -i <container> python /opt/nandi/app.py --stdio` for each session (no port, no bearer token). The host gives it 30 s to answer `initialize`, times calls out 30 s after the shell timeout (60 s by default, `mcp_client::call_timeout`), and truncates results over 100,000 characters.
- **Where:** `runtimes/nandi-runtime/app.py`, `src-tauri/src/mcp_client.rs` (`McpSession::connect(container_id, call_timeout)`, `TokioChildProcess`), used by `services/runtimes.rs` and the orchestrator.
- **Preconditions:** Docker, `nandi-runtime` built, a native agent with `allowed-tools` including these tools and a working model.
- **Steps:** 1. Ask the agent to run `echo hi`, write `a.txt`, list the folder, read it back. 2. Ask it to `sleep 40`; then set `shell_timeout_s: 120` and ask again. 3. Ask it to read `../../etc/passwd`.
- **Expected:** Results appear as tool steps in the run; `a.txt` exists in `~/.nandi/local/work/<agent>/`; the first sleep fails with a timeout reported by the server, the second finishes; paths outside the workdir are refused.
- **Edge / negative cases:** `app.py` started without `--stdio` exits with "Run with --stdio (Nandi starts it with docker exec)"; `docker` not on PATH ("Docker is not installed"); output larger than 100k chars truncated; container killed mid-call (transport error, not tool error).
- **Automation:** Covered by: `mcp_client.rs` tests; `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images) `the_native_tool_server_is_confined`: the four tools over `docker exec` stdio, a path escape refused, the agent folder and the root read-only, no direct internet, no keys in the env | Layer: api (Docker) | Gap: the shell timeout (`shell_timeout_s`) and output truncation are unit-tested only; no Python tests for `app.py`.
- **Needs relay:** no
- **Status:** Shipped

### RT-07 — Built-in tool catalog and On/Off switch

- **What:** The tool catalog lists built-in tools (`shell`, `read_file`, `write_file`, `list_dir`, delegation), connector and pilot tools; a built-in tool can be switched off (confirmation asked, agents lose it) and on; usage stats are available.
- **Where:** Library → Browse, Tools filter (On/Off switch in `src/components/library/LibraryItemAction.tsx`); `src/hooks/useTools.ts`; `src-tauri/src/commands/tools.rs` (`list_tools`, `get_tool`, `set_tool_enabled`, `get_tool_usage_stats`), `src-tauri/src/services/tools/` (`builtin.rs`, `catalog.rs`). HTTP `GET /api/tools`, `GET /api/tools/usage`, `GET /api/tools/{id}`, `PUT /api/tools/{id}/enabled`. Event `tool://catalog_changed`.
- **Preconditions:** None.
- **Steps:** 1. Open Library, filter to Tools, find "Run shell commands" (`shell`). 2. Switch it Off, click Switch off. 3. Restart the app. 4. Switch it On.
- **Expected:** Off opens "Switch off Run shell commands?" ("Agents that use … won't be able to until you switch it back on."). After Off, `tool_catalog.enabled = 0` and survives a catalog rebuild and restart; On needs no confirmation. Delegation tool has `requires_approval: true` by default.
- **Edge / negative cases:** unknown tool id (error); stopped pilot keeps its tools listed; uninstalled connector drops them.
- **Automation:** Covered by: `src-tauri/src/services/tools/tests.rs`, `builtin.rs` tests, `src/components/library/LibraryView.test.tsx` | Layer: api | Gap: none significant.
- **Needs relay:** no
- **Status:** Shipped

## Coding-agent runtimes (harnesses)

### RT-08 — Runtime cards and detection (Nandi, Claude Code, Codex, Gemini CLI)

- **What:** The Coding agents group shows a Nandi card ("Built in. Uses the model chosen above.") and one card per detected CLI with install state and version; "Detect again" re-detects; before detection answers placeholder cards show "Checking…", then "Couldn’t check" after 15 s.
- **Where:** Settings → Models & runtimes → Coding agents. `src/components/settings/sections/RuntimesSection.tsx` (`RuntimesPanel`), `src/components/harness/HarnessCard.tsx` (`HarnessCard`, `NandiRuntimeCard`, `PendingRuntimeCard`), `src-tauri/src/commands/harnesses.rs` (`list_harnesses`, `detect_harnesses`), `services/harness/detection.rs`. HTTP `GET /api/harnesses`, `POST /api/harnesses/detect`. Event `harness://detected`.
- **Preconditions:** Optionally `claude`, `codex`, `gemini` installed on the host (used for sign-in detection only).
- **Steps:** 1. Open Settings › Models & runtimes with none installed. 2. Install Claude Code, click Detect again.
- **Expected:** Not installed: "Not found. Install Claude Code to run agents in it." Installed: "On this computer for sign-in: version x.y.z". Detection results stored in SQLite and returned by `GET /api/harnesses`.
- **Edge / negative cases:** CLI whose `--version` hangs (15 s timeout); detection error shows a red alert "Couldn’t check the runtimes on this computer: …"; RTL layout.
- **Automation:** Covered by: `src/components/harness/harness-ui.test.tsx`, `src/components/settings/SettingsView.test.tsx`, Rust tests in `services/harness/detection.rs` | Layer: api + unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RT-09 — Sign-in status per runtime card ("Ready")

- **What:** Each card says whether it runs in the sandbox (image built), what it signs in with (account login with plan/org, AWS Bedrock profile/region, Vertex, Azure, gateway, API key), and whether that sign-in reaches the sandbox; Ready means both. An expired AWS SSO session shows the `aws sso login --profile <p>` command and "Re-check sign-in"; a key is only offered as "Use an API key instead" (jumps to that provider's key row).
- **Where:** Settings → Models & runtimes → Coding agents → each card. `HarnessCard.tsx`, `src/components/harness/runtime-options.ts`, `services/harness/detection.rs` (`auth_of`, `container_auth`, `provider_and_auth`). `HarnessHealth.auth`, `containerAuthenticated`.
- **Preconditions:** A CLI installed and/or signed in on the host; image built or not.
- **Steps:** 1. Claude Code logged in on host, image not built. 2. Build image. 3. Log Claude Code out, set an Anthropic key. 4. Use an expired SSO profile in Claude Code's Bedrock settings.
- **Expected:** 1. "The sandbox image isn’t built yet: build it above." (status "Needs the sandbox"). 2. "Runs in Nandi’s sandbox container", "Signs in with ...", "Your sign-in reaches the sandbox" (status "Ready"). 3. "Signs in with an API key". 4. "Your AWS SSO session expired. On this computer, run:" plus the command; Re-check sign-in re-runs detection. With nothing usable: "The sandbox can’t sign in yet: sign in on this computer, or set up AWS in Cloud credentials" (status "Needs sign-in").
- **Edge / negative cases:** ChatGPT or Google OAuth logins (not usable in the container); host Claude token with under 15 minutes left (not usable); macOS keychain item `Claude Code-credentials` (detected without prompting).
- **Automation:** Covered by: `harness-ui.test.tsx`, `detection.rs` tests | Layer: unit + manual (real sign-ins) | Gap: no end-to-end check that "Ready" really runs.
- **Needs relay:** no
- **Status:** Shipped

### RT-10 — Test connection for a runtime

- **What:** "Test connection" checks detection, then opens and closes a one-turn ACP session in a throwaway folder (in the harness container when its image is built, else on the host), returning OK/failed, latency, version and provider.
- **Where:** Settings → Models & runtimes → Coding agents → card → Test connection. `commands/harnesses.rs` (`test_harness`), `services/harness/mod.rs` (`HarnessService::test`, `test_config`). HTTP `POST /api/harnesses/{id}/test`.
- **Preconditions:** CLI detected; a sign-in.
- **Steps:** 1. Click Test connection on Claude Code. 2. Repeat with no sign-in. 3. Repeat with the CLI uninstalled.
- **Expected:** "Connection works" or "Connection failed" with the error; uninstalled gives `NotInstalled`. Temp folder `nandi-harness-test-<uuid>` under the OS temp dir; container `nandi-harness-test-<12 chars>`. Request timeout 90 s.
- **Edge / negative cases:** image missing (test runs the CLI on the host: documented exception to "agents run only in the container"); the test container is not put on `nandi-agents` (`test_config` sets no network), so it has the default bridge's direct internet, unlike a real run (checklist "Known gaps"); custom harness without a registered command.
- **Automation:** Covered by: `harness-ui.test.tsx` | Layer: manual (real CLI and sign-in) | Gap: no backend test of `test`.
- **Needs relay:** no
- **Status:** Shipped

### RT-11 — Runtime Details dialog

- **What:** Details shows the executable path, version, config folder, provider, "Signed in (host)", "Signed in (container)" (or "No: sign in on this computer (your account or AWS); an API key is the last resort"), "Container image built", last check time and the last test result.
- **Where:** Card → Details. `src/components/harness/HarnessHealthDialog.tsx`, `commands/harnesses.rs` (`get_harness_health`). HTTP `GET /api/harnesses/{id}/health`.
- **Preconditions:** Detection ran.
- **Steps:** 1. Open Details for each card. 2. Run Test, reopen.
- **Expected:** Fields filled from `HarnessHealth`; test result shown after a test; no secret values displayed.
- **Edge / negative cases:** not installed (empty path); unknown version shows "unknown".
- **Automation:** Covered by: `harness-ui.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RT-12 — Make a runtime the default

- **What:** "Make default" marks one CLI runtime as default (badge "Default"); with none marked, the Nandi card is default. The stored default's model is used for that runtime's runs when the agent sets none.
- **Where:** Card → Make default. `commands/harnesses.rs` (`set_harness_default`, `clear_harness_default`, `get_harness_default`), table `harness_preferences`. HTTP routes under `/api/harnesses` (default).
- **Preconditions:** A detected CLI.
- **Steps:** 1. Click Make default on Codex. 2. Reload the page. 3. Create a new agent.
- **Expected:** Codex card shows Default, Nandi card loses it; persists across restart.
- **Edge / negative cases:** the UI always sends `model: null`, so no default model is ever set from the UI; nothing reads the default runtime when creating agents.
- **Automation:** Covered by: `harness-ui.test.tsx`, `services/harness/mod.rs` tests | Layer: api | Gap: effect on new agents untested (none exists).
- **Needs relay:** no
- **Status:** Partial (badge only; per-runtime model picker not built: `task-x-models-and-runtimes.md` Wave A)

### RT-13 — Choose an agent's runtime

- **What:** On the agent's Settings tab the runtime is chosen by radio (Nandi, Claude Code, Codex, Gemini CLI; uninstalled ones disabled and marked "Not installed · set up in Settings › Models & runtimes"); a CLI runtime shows "Where it runs" with the fixed text "In an isolated container. Agents never run directly on this computer." (there is nothing to choose) and a free-text "Model (optional)" (`runtime_config.model`, placeholder "Default for <runtime>"); a note with "Add a key" appears when the sign-in doesn't reach the container.
- **Where:** Agent page → Settings tab. `src/components/agents/AgentSettingsTab.tsx`, `src/components/harness/runtime-options.ts`. Saved to `~/.nandi/repo/.nandi/agents/<id>/skill.md` (`runtime:`, `runtime_config:`).
- **Preconditions:** An agent; a CLI detected.
- **Steps:** 1. Switch an agent to Claude Code, type `sonnet` as model, save. 2. Open skill.md.
- **Expected:** `runtime: claude-code`, `runtime_config.model: sonnet`; unknown `runtime_config` keys kept.
- **Edge / negative cases:** CLI not installed (option disabled unless already chosen); switching back to Nandi shows the provider/model fields.
- **Automation:** Covered by: agent page tests (`src/components/agents/agent-page-model.test.ts`), `runtime_router.rs` save tests | Layer: ui-e2e / api | Gap: no UI test for the runtime radio.
- **Needs relay:** no
- **Status:** Shipped

### RT-14 — Runtime routing for a run (native vs harness, opt-in fallback)

- **What:** Each run resolves the agent's runtime: native agents use Nandi's loop; CLI agents get a plan (image present, CLI detected, workdir, provider credentials, gateway) or fail with a reason; `fallback: native` runs natively when the harness can't start, otherwise the run fails.
- **Where:** `src-tauri/src/services/orchestrator/runtime_router.rs` (`resolve_runtime`, `plan`, `provider`, `credentials`), `services/orchestrator.rs` (`harness_run`, `execute_harness_run`). Docs `docs/developer/runtime-routing.md`.
- **Preconditions:** A `runtime: claude-code` agent; variants with/without image and sign-in.
- **Steps:** 1. Run with no harness image. 2. Run with image but no sign-in. 3. Add `fallback: native` and repeat 2. 4. Run with everything set.
- **Expected:** 1–2: run fails with a plain reason (image missing / sign-in needed) before any container starts. 3: run completes natively. 4: run row has `runs.runtime = claude-code`. Provider order: `runtime_config.provider`, else `bedrock` with a gateway, else the CLI's own config; `.nandi/llm.yaml` default provider sits between agent and CLI.
- **Edge / negative cases:** Studio/Ask and agents without `runtime:` are always native; `runtime_config.gateway: litellm` starts the gateway if not serving that model; a CLI model a policy's `models.allow` refuses makes the run unavailable even with `fallback: native` (PO-13).
- **Automation:** Covered by: Rust tests in `runtime_router.rs` (20 tests) | Layer: unit + manual | Gap: no Docker-backed run test.
- **Needs relay:** no
- **Status:** Shipped

### RT-15 — Harness session in the container

- **What:** A CLI run starts `docker run -i --rm --init --name nandi-harness-<agent> --network=nandi-agents` on `ghcr.io/vaultdbai/nandi-harness:sandbox-6` with the agent workdir at `/agent/workdir` and shared folders (after `relay::ensure_ready(RelayImage::Harness)`), with the egress proxy env and its own proxy token (SB-11, SB-15), speaks ACP over stdio, sends one prompt (system prompt in an `<instructions>` block), streams updates, then ends; stderr last 20 lines explain failures.
- **Where:** `src-tauri/src/services/harness/adapter.rs`, `acp.rs`, `claude_code.rs`, `codex.rs`, `gemini_cli.rs`; `runtimes/nandi-harness/Dockerfile`, `entrypoint.sh`, `codex-config.toml`. Events `harness://session_started`, `harness://turn_delta`, `harness://turn_finished`, `harness://session_ended`, `harness://error`.
- **Preconditions:** Docker, `nandi-harness` built, a working sign-in for the CLI.
- **Steps:** 1. Ask a claude-code agent to create `hello.txt`. 2. `docker ps` and `docker inspect nandi-harness-<tag>-<agent> --format "{{.HostConfig.NetworkMode}}"` during the run. 3. Check the workdir after.
- **Expected:** Container exists only during the run (`--rm`) and is on `nandi-agents`; `hello.txt` in `~/.nandi/local/work/<agent>/`; run completes with text and tool steps. Codex runs with `sandbox_mode = "danger-full-access"` copied into its tmpfs home by the entrypoint.
- **Edge / negative cases:** CLI exits early (stderr tail in the error); Gemini via `gemini --acp`, Codex via `codex-acp`, Claude via `claude-agent-acp`.
- **Automation:** Covered by: `acp.rs`, `adapter.rs`, `claude_code.rs`, `codex.rs`, `gemini_cli.rs` tests | Layer: manual (Docker + real sign-in) | Gap: container run on a subscription token not yet verified end to end.
- **Needs relay:** no
- **Status:** Known issue (container turn on subscription token unverified)

### RT-16 — Host placement retired

- **What:** A skill.md with `placement: host` was read, run in the container and rewritten to `container` on save. The `placement` key and `RuntimePlacement::Host` no longer exist.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### RT-17 — Credentials handed to the harness container (company identity first, keys last)

- **What:** The container gets only env values for one session, passed as `-e NAME` (value from the docker process env, never on a command line): Bedrock (team/agent choice) as temporary STS keys plus `CLAUDE_CODE_USE_BEDROCK=1`, `AWS_REGION` (or `AWS_BEARER_TOKEN_BEDROCK`); else Claude Code's own sign-in (`CLAUDE_CODE_OAUTH_TOKEN` from env/settings or the host login's access token with 15+ min left); else `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` from the keyring or env. No `~/.claude`, `~/.aws`, `~/.codex`, `~/.gemini` mounts. With nothing to sign in with, the run is refused before the container starts.
- **Where:** `services/orchestrator/runtime_router.rs` (`credentials`), `services/harness/claude_code.rs` (`env`), `services/harness/detection.rs` (`claude_code_oauth_token`), `services/harness/aws_auth.rs` (`bedrock_credentials`). Docs `sign-in.md`, `harness-container.md`.
- **Preconditions:** Docker + image; one of: AWS profile, Claude Code login, API key.
- **Steps:** 1. Run with only a Claude Code login. 2. With `.nandi/llm.yaml provider: bedrock` and an SSO profile. 3. With an expired SSO session. 4. With nothing.
- **Expected:** `docker inspect` shows env names with values only inside the container; no host config mounts. 3 fails with `aws sso login --profile <p>`. 4 refused up front with guidance (fix `918d712`).
- **Edge / negative cases:** refresh token never leaves the host; ChatGPT/Google OAuth not supported in the container.
- **Automation:** Covered by: `claude_code.rs`, `aws_auth.rs`, `runtime_router.rs`, `detection.rs` tests | Layer: unit + manual | Gap: raw keys are still readable by agent code in the container (moved to Wave 3, model access through the gateway; see Gaps and planned).
- **Needs relay:** no
- **Status:** Shipped

### RT-30 — ACP permission answers from `allowed-tools`

- **What:** ACP permission requests not covered by `requires_approval` are answered automatically from the agent's tools as ACP kinds: `shell` → execute, `read_file` and `list_dir` → read/search, `write_file` → edit/delete/move, `fetch` → fetch; other names are matched against the call's title. A request matching none of the allowed tools is rejected; if every listed tool is unknown, the agent is read-only. No `allowed-tools` means everything is allowed (the container is the sandbox), except tools a policy denies: those are rejected whatever else allows them (`PermissionPolicy::Except`, PO-13).
- **Where:** `services/harness/acp.rs` (`PermissionPolicy`, `from_tools`), `services/orchestrator/runtime_router.rs` (`TOOL_KINDS`, `ToolGrant`), `services/harness/adapter.rs` (calls `PermissionPolicy::from_tools`).
- **Preconditions:** A CLI agent with `allowed-tools: [read_file]` and no `requires_approval`.
- **Steps:** 1. Ask the agent to edit a file.
- **Expected:** The edit is rejected by the permission policy; reading works.
- **Edge / negative cases:** empty `allowed-tools`; unknown tool kinds; a policy with `tools: { deny: [shell] }` on an agent with no `allowed-tools` (command requests rejected, edits still allowed).
- **Automation:** Covered by: `acp.rs` tests (`permissions_follow_the_policy_and_the_approvals`), `runtime_router.rs` (`a_policy_s_tools_reach_the_cli`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RT-19 — Turn limit for harness runs (`max_turns`)

- **What:** A harness run counts tool calls against `runtime_config.max_turns`, else skill.md `max_turns` (default 50); going over sends `session/cancel` and fails the run.
- **Where:** `services/orchestrator/runtime_router.rs` (`max_turns` and the step counter), `services/orchestrator.rs` (`execute_harness_run`).
- **Preconditions:** A working CLI agent.
- **Steps:** 1. Set `runtime_config.max_turns: 2`. 2. Ask for a task needing many tool calls.
- **Expected:** Run fails after the third tool call with a turn-limit reason; container removed.
- **Edge / negative cases:** `max_turns` absent (50).
- **Automation:** Covered by: `runtime_router.rs` tests | Layer: unit + manual | Gap: a real harness run is manual.
- **Needs relay:** no
- **Status:** Shipped

### RT-20 — Harness run steps, usage and evidence

- **What:** ACP updates are written as native steps (`agent://token`, `agent://tool_call`, `agent://tool_result`) so chat, history and run details render them; usage and `runs.runtime` are recorded; up to 5,000 events per session are stored.
- **Where:** `services/harness/event_translation.rs` (`TurnTranslator`), `services/harness/evidence.rs` (`session_started`, `record_event`, `session_ended`, `attribute_run`), tables `harness_sessions`, `harness_events`.
- **Preconditions:** A completed harness run.
- **Steps:** 1. Open the run's detail view. 2. `GET /api/harness-sessions/{id}/events`.
- **Expected:** Text, thinking and tool calls appear in the run timeline; `runs.runtime` = the CLI id; session row has input/output tokens.
- **Edge / negative cases:** event cap reached; thinking blocks.
- **Automation:** Covered by: `event_translation.rs`, `evidence.rs` tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RT-21 — Recent harness sessions list and Stop

- **What:** Advanced shows recent harness sessions (below Enterprise config) with status (Starting, Running, Finished, Failed, Stopped) and token counts ("N in / M out tokens"); Stop kills a live session (CLI and container stop, run fails).
- **Where:** Settings → Models & runtimes → Advanced (sessions list). `src/components/harness/HarnessSessionList.tsx`, `commands/harness_sessions.rs` (`list_harness_sessions`, `get_harness_session`, `get_harness_events`, `kill_harness_session`). HTTP `GET /api/harness-sessions`, `GET|DELETE /api/harness-sessions/{id}`, `GET /api/harness-sessions/{id}/events`.
- **Preconditions:** At least one harness run.
- **Steps:** 1. Start a long harness run. 2. Open Settings › Models & runtimes, expand Advanced, click Stop on the running session.
- **Expected:** Row goes to Stopped; run fails; container gone. Empty state: "No harness sessions yet".
- **Edge / negative cases:** Cancelling the run instead closes the session row via a drop guard; sessions left open by a crash are closed on start (`close_orphaned_sessions`).
- **Automation:** Covered by: `src/components/harness/HarnessSessionList.test.tsx`, `evidence.rs` tests | Layer: api | Gap: kill path needs Docker.
- **Needs relay:** no
- **Status:** Shipped

### RT-22 — Harness health monitor

- **What:** Every 5 minutes health is re-detected and `harness://health_changed` emitted when install, version, host/container sign-in, provider or error changes.
- **Where:** `services/harness/health_monitor.rs`, `services/harness/mod.rs` (`same_health`).
- **Preconditions:** App running.
- **Steps:** 1. Log Claude Code out on the host; wait up to 5 min.
- **Expected:** Card updates without Refresh.
- **Edge / negative cases:** none.
- **Automation:** Covered by: none | Layer: unit (inject clock) | Gap: no tests in `health_monitor.rs`.
- **Needs relay:** no
- **Status:** Shipped

### RT-23 — Custom ACP harness (`runtime: custom:<id>`)

- **What:** Any ACP-speaking executable can be an agent runtime as `custom:<id>`, using the command/args stored in `harness_preferences`; it gets the run's credentials under the usual names.
- **Where:** `services/harness/custom.rs`, `services/harness/mod.rs` (`custom_command`).
- **Preconditions:** A row in `harness_preferences` with `command` and `args` (no UI or command writes these).
- **Steps:** 1. Set `runtime: custom:my-agent` without registering. 2. Run.
- **Expected:** Error "Custom harness 'custom:my-agent' has no command: register it first".
- **Edge / negative cases:** registration only by editing SQLite.
- **Automation:** Covered by: `custom.rs` tests, `runtime_router.rs` parse tests | Layer: unit | Gap: no registration path.
- **Needs relay:** no
- **Status:** Partial (no way to register a command)

### RT-24 — `verify --agent <id>` CLI check

- **What:** Prints a pass/fail report of an agent's end-to-end prerequisites: skill.md parses, model, `shell` allowed (and whether it needs approval), Docker running, runtime image present and current, `state.db` exists with this build's schema (v2; another version fails: "… has schema vN, but this build of Nandi uses v2"), API key stored for the model's provider; exits 1 on any failure. No LLM calls, no containers.
- **Where:** `nandi verify --agent <id>` (`src-tauri/src/cli.rs`, `src-tauri/src/verify.rs`).
- **Preconditions:** Built binary.
- **Steps:** 1. `nandi verify --agent echo-agent`. 2. Stop Docker, rerun. 3. `nandi verify` without `--agent`. 4. `nandi --agent x`.
- **Expected:** Lines with pass/fail/note markers; "All checks passed." exit 0, or "N check(s) failed." exit 1. 3: "verify needs --agent <id>". 4: "--agent is only valid with verify".
- **Edge / negative cases:** model via Claude Code sign-in or the trial is reported as missing key (only `require_api_key` is checked); harness image and CLI runtimes not checked.
- **Automation:** Covered by: `cli.rs` tests (argument parsing) | Layer: unit | Gap: no tests in `verify.rs`.
- **Needs relay:** no
- **Status:** Partial (ignores Claude Code sign-in, trial and CLI runtimes)

### RT-25 — Headless and server modes

- **Duplicate:** tested under PL-02, PL-03. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RT-26 — E2E smoke prerequisites script

- **Duplicate:** tested under QA-05. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RT-27 — Canned turn engine (test fixture)

- **What:** A scripted engine replays a script stored in the `settings` table (`canned_script`), re-read each step, so run state and events can be exercised without an LLM.
- **Where:** `src-tauri/src/engine.rs`, seeded in `db.rs`; used by tests and `http/manual_steps.rs`.
- **Preconditions:** Test or dev setup.
- **Steps:** 1. Set `canned_script` to a JSON script, run the agent through the path that uses it.
- **Expected:** Text streamed with ~35 ms per token, scripted tool calls executed.
- **Edge / negative cases:** malformed script.
- **Automation:** Covered by: `engine.rs` tests, `services/a2a/tests.rs` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RT-28 — List Docker containers command

- **What:** Lists all containers (running and stopped) with short id, name, image, state, status.
- **Where:** `src-tauri/src/commands/docker.rs` (`list_containers`). No UI uses it.
- **Preconditions:** Docker.
- **Steps:** 1. Call `commands.listContainers()` from devtools.
- **Expected:** Array of `ContainerInfo`; error "Failed to connect to Docker: ..." without Docker.
- **Edge / negative cases:** unnamed containers ("unnamed").
- **Automation:** Covered by: `commands/docker.rs` tests | Layer: unit | Gap: not surfaced in UI.
- **Needs relay:** no
- **Status:** Partial (backend only)

## Models, providers and keys

### MD-01 — Add a provider key (tested before saving)

- **What:** Provider keys (Anthropic, OpenAI, DeepSeek) are write-only: a pasted key is tested against the provider and stored only if it works; when the provider can't be reached at all the user may save it untested; "Get a key" opens the provider's page.
- **Where:** Settings → Models & runtimes → Provider keys → provider row. `src/components/models/ProviderKeyForm.tsx`, `src/components/preferences/LlmKeysSection.tsx` (`ProviderRow`), `commands/llm_keys.rs` (`test_llm_key`, `set_llm_api_key`), `services/llm_keys.rs`. HTTP `POST /api/llm-keys/{provider}/test` (body `{apiKey?}`), `PUT /api/llm-keys/{provider}`, `GET /api/llm-keys`.
- **Preconditions:** A real key for one provider.
- **Steps:** 1. In Settings › Models & runtimes → Provider keys, paste a valid key, click Save (Replace when a key is stored). 2. Paste an invalid key, Save. 3. Go offline, paste a key, Save, then click "Save without testing".
- **Expected:** 1. Toast "API key saved"; row shows configured; keyring entry `nandi.llm.<provider>`; key never returned to the webview. 2. Test result shows invalid key; nothing stored. 3. Untested save stored.
- **Edge / negative cases:** whitespace-only key; key with trailing spaces trimmed; empty key via API removes it.
- **Automation:** Covered by: `src/components/preferences/LlmKeysSection.test.tsx`, `services/llm_keys.rs` tests | Layer: api | Gap: real provider calls are manual.
- **Needs relay:** no
- **Status:** Shipped

### MD-02 — Key test result classification

- **What:** A key test reports OK with latency, or a failure kind: invalid key (401/403), no quota (402, 429 or 400 with quota/credit text), rate limited (429), unreachable (network, 5xx), no key, other; raw provider answer only under Details.
- **Where:** `src/components/models/KeyTestResult.tsx`, `services/llm_keys.rs` (`key_test_failure`), `src-tauri/src/llm/probe.rs`. Also "Test" on a stored key (`POST /api/llm-keys/{provider}/test` with no body).
- **Preconditions:** Keys in the various states.
- **Steps:** 1. Test a revoked key. 2. Test an OpenAI key with no credit. 3. Test with no stored key.
- **Expected:** Matching messages; `failure` = `invalidKey` / `noQuota` / `noKey`.
- **Edge / negative cases:** 15 s probe timeout; body text truncated to 300 chars.
- **Automation:** Covered by: `llm_keys.rs` tests, `probe.rs` tests | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-03 — Remove a provider key

- **What:** Remove asks "Remove your <provider> key?", saying "Agents using these models will switch to <provider>." or "Agents will stop working until you add a model." (Cancel / Remove key), then deletes the keyring entry.
- **Where:** Provider row → Remove. `LlmKeysSection.tsx` (`RemoveKeyButton`).
- **Preconditions:** A stored key.
- **Steps:** 1. Remove the only key. 2. With two keys, remove one.
- **Expected:** Dialog with the "stop working" vs "switch to" line; toast "API key removed"; status line updates.
- **Edge / negative cases:** Cancel keeps the key.
- **Automation:** Covered by: `LlmKeysSection.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-04 — Default model: pick provider, type any model

- **What:** The default model for Nandi's own calls is a provider select (Anthropic, OpenAI, DeepSeek, Ollama) plus a free-text model name; picking a provider fills in its usual model (for Ollama one it has); Save writes `<provider>:<model>` to `settings.yaml` in the repo; a provider without a key says where to add it.
- **Where:** Settings → Models & runtimes → Nandi (built in) → Default model. `LlmKeysSection.tsx` (`DefaultModelField`), `src/lib/models.ts`, `commands/llm_keys.rs` (`get_default_model`, `set_default_model`). HTTP `GET|PUT /api/default-model`.
- **Preconditions:** None.
- **Steps:** 1. Pick OpenAI, type `gpt-5.1`, Save. 2. Inspect `~/.nandi/repo/.nandi/settings.yaml`. 3. Pick a provider with no key.
- **Expected:** Toast "Default model saved"; settings file `default_model: gpt:gpt-5.1` committed through the repo; needs-key hint for 3. Save disabled when unchanged or empty.
- **Edge / negative cases:** invalid model string via API ("Invalid model ... expected '<provider>:<model>'"); unknown prefix (`nandi:` is reserved and rejected); choosing `ollama:` turns the trial off for good.
- **Automation:** Covered by: `LlmKeysSection.test.tsx`, `src/lib/models.choices.test.ts`, `src/lib/models.test.ts`, `llm_keys.rs` tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-05 — Suggested models from the provider's own list

- **What:** With a key saved, the model field's suggestions come from the provider's live model list (Anthropic, OpenAI, DeepSeek), newest first, OpenAI embedding/audio/image/moderation models left out; without a key or when unreachable, built-in hints (or Ollama's installed models) are shown; any model can still be typed.
- **Where:** Same field (datalist `llm-model-suggestions`). `src/hooks/useLlmKeyStatus.ts` (`useProviderModels`), `commands/llm_keys.rs` (`list_provider_models`), `llm/probe.rs` (`model_ids`). HTTP `GET /api/llm-keys/{provider}/models`.
- **Preconditions:** A provider key.
- **Steps:** 1. Pick Anthropic with a key, focus the model field. 2. Remove the key.
- **Expected:** Live IDs listed; built-in hints after removal.
- **Edge / negative cases:** provider down (fallback to hints); no `text-embedding-*`/`whisper`/`dall-e`/`omni-moderation` in OpenAI suggestions.
- **Automation:** Covered by: `LlmKeysSection.test.tsx`, `probe.rs` tests | Layer: api | Gap: live listing is manual.
- **Needs relay:** no
- **Status:** Shipped

### MD-06 — "Which model calls use, and why" line

- **What:** Under the default model a line says what calls really use: "Calls use <model> on your free tokens for now.", "No <provider> key yet, so calls use <model>.", "Calls use <model> instead.", or a warning "No model can be called: pick one and add its key below."
- **Where:** `LlmKeysSection.tsx` (`data-testid="model-reason"`), `src/lib/models.ts` (`modelReason`), `DefaultModelStatus.effectiveModel`.
- **Preconditions:** Various key states.
- **Steps:** 1. Default `claude:...`, only an OpenAI key. 2. No keys, trial used up/off.
- **Expected:** 1. "No Anthropic key yet, so calls use GPT-5." 2. Warning line.
- **Edge / negative cases:** Claude Code sign-in counts as Anthropic access.
- **Automation:** Covered by: `LlmKeysSection.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-07 — Model resolution and fallback order for every call

- **What:** Every model call uses the requested model when its provider is usable (key, Ollama needs none, Claude Code sign-in counts for `claude:`), else the first usable of `claude:claude-sonnet-5`, `gpt:gpt-5`, `deepseek:deepseek-flash`, else the free trial while tokens remain, else an error naming what to add. Default comes from `NANDI_LLM_MODEL`, else the team's `.nandi/llm.yaml` default provider (native kinds), else settings.
- **Where:** `services/llm_keys.rs` (`usable_model`, `resolve_model`), `llm/mod.rs` (`stream`, `parse_model`), `services/config/repo_config.rs` (`default_model_with`).
- **Preconditions:** Key combinations.
- **Steps:** 1. Agent with `model: deepseek:deepseek-chat`, only Anthropic key; run. 2. No keys, trial used up; run.
- **Expected:** 1. Call goes to Claude; app log "No DeepSeek key for ...; using claude:claude-sonnet-5 (Anthropic key)". 2. Error "Your 100,000 free tokens are used up. Add your own key (Anthropic, OpenAI, or DeepSeek) in Settings › Models, …". Without trial in the build: "No LLM API key is set ..." (both still say "Settings › Models", not "Models & runtimes").
- **Edge / negative cases:** `NANDI_LLM_MODEL` invalid value ignored; `NANDI_LLM_MODEL` is not read.
- **Automation:** Covered by: `llm_keys.rs` tests, `repo_config.rs` tests, `llm/mod.rs` tests | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-08 — Provider clients (Anthropic, OpenAI, DeepSeek, Ollama)

- **What:** Calls stream text (5 min overall timeout for non-streaming, 120 s idle for streams, 15 s connect), default 4096 max tokens; Ollama talks to `localhost:11434`; dropping the call aborts the HTTP request.
- **Where:** `src-tauri/src/llm/anthropic.rs`, `openai.rs`, `deepseek.rs`, `ollama.rs`, `mod.rs`.
- **Preconditions:** Key or Ollama.
- **Steps:** 1. Chat with an agent on each provider. 2. Cancel mid-stream.
- **Expected:** Streamed tokens in chat; cancellation stops the request; `llm.log` entry per call.
- **Edge / negative cases:** provider 5xx; idle stream longer than 120 s errors.
- **Automation:** Covered by: tests in each `llm/*.rs` | Layer: unit + manual | Gap: real-provider runs are manual.
- **Needs relay:** no
- **Status:** Shipped

### MD-09 — LLM usage and cost log (`llm.log`)

- **What:** Every model call (native, Studio/Ask, proxy) appends a JSON line with timestamp, source, agent id, model actually used, status, input/output tokens, duration, cost (DeepSeek only) and error.
- **Where:** `~/.nandi/local/logs/llm.log`; `llm/mod.rs` (`LlmAuditEntry`), `src-tauri/src/audit.rs` (`AuditLog::Llm`). Viewed in the Logs viewer (other area).
- **Preconditions:** Any model call.
- **Steps:** 1. Make a call. 2. Tail the file. 3. Make a failing call.
- **Expected:** One line per call; `status: "error"` with message for 3; no keys in the log.
- **Edge / negative cases:** `cost_usd` absent for Anthropic, OpenAI, Ollama, Claude Code.
- **Automation:** Covered by: `audit.rs` tests | Layer: unit | Gap: cost only computed for DeepSeek.
- **Needs relay:** no
- **Status:** Shipped

### MD-10 — Claude models through Claude Code's sign-in (Agent SDK bridge)

- **What:** Nandi's own `claude:` calls go through the user's Claude Code when it can sign in (Claude Code + Node found, and a login or the team's Bedrock route), before an Anthropic key; with nothing else set up every call uses it. Each call is a plain one-turn completion (no tools, no MCP, no settings/hooks, empty folder); Nandi's tools are described in the prompt and parsed from `<tool_call>` blocks; model names map to aliases (`claude-opus-5` → `opus`).
- **Where:** `src-tauri/src/llm/claude_code.rs`, `services/harness/claude_sdk.rs` (`usable`, `complete`, `bedrock_route`), `runtimes/claude-sdk/bridge.mjs`, `runtimes/claude-sdk/package.json`. SDK `@anthropic-ai/claude-agent-sdk@0.3.304` installed on first use into `~/.nandi/local/runtimes/claude-sdk` with `npm install --omit=optional`.
- **Preconditions:** `claude` and `node`/`npm` on PATH, Claude Code logged in (or `.nandi/llm.yaml` Bedrock + AWS profile). No Anthropic key.
- **Steps:** 1. Remove all keys, make sure the trial is off. 2. Chat with Ask. 3. Inspect `~/.nandi/local/runtimes/claude-sdk`.
- **Expected:** Answer arrives; SDK installed at the pinned version; `llm.log` model `claude:...`; Settings shows which sign-in Claude calls use (MD-11).
- **Edge / negative cases:** npm missing ("npm (it comes with Node.js)" not installed error); Bedrock route passes `CLAUDE_CODE_USE_BEDROCK=1`, `AWS_PROFILE`, `AWS_REGION`; stale login is found but fails (trial stays on).
- **Automation:** Covered by: `llm/claude_code.rs`, `services/harness/claude_sdk.rs` tests | Layer: manual | Gap: no bridge test.
- **Needs relay:** no
- **Status:** Shipped

### MD-11 — Claude sign-in shown on the Anthropic key row

- **What:** When Claude calls can sign in through Claude Code, the Anthropic row says which sign-in they use (account login or Bedrock profile/region) and keeps the key form behind "Use an API key instead".
- **Where:** Provider keys → Anthropic. `LlmKeysSection.tsx` (`ProviderRow`), `DefaultModelStatus.claudeSignIn` (`services/llm_keys.rs` `claude_sign_in`).
- **Preconditions:** Claude Code logged in.
- **Steps:** 1. Open Provider keys. 2. Click "Use an API key instead".
- **Expected:** Sign-in line; key form revealed.
- **Edge / negative cases:** Claude Code not usable: plain key form.
- **Automation:** Covered by: `LlmKeysSection.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-12 — Ollama (local models)

- **What:** The "Ollama (on this computer)" row says whether Ollama is running and which models it has, with "Check again" and "Install Ollama from ollama.com"; Ollama needs no key; its installed models are suggested in the default model field.
- **Where:** Provider keys → Ollama. `LlmKeysSection.tsx` (`OllamaRow`), `commands/llm_keys.rs` (`detect_ollama`), `llm/probe.rs` (`ollama_models`, 2 s timeout). HTTP `GET /api/ollama`.
- **Preconditions:** Ollama installed (or not).
- **Steps:** 1. With Ollama stopped, open the row. 2. Start Ollama, Check again. 3. Make `ollama:llama3` the default and chat.
- **Expected:** "Not running" then "Running · N models"; calls go to `localhost:11434`; trial turns off.
- **Edge / negative cases:** model not pulled (provider error); no Ollama container (reserved, not built).
- **Automation:** Covered by: `LlmKeysSection.test.tsx`, `llm/ollama.rs` tests | Layer: manual | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-13 — Free tokens (trial gateway)

- **What:** Every build has a free trial on `deepseek:deepseek-flash` through Nandi's trial gateway (`https://trial.vaultdb.ai`, `NANDI_TRIAL_GATEWAY_URL` overrides it). The DeepSeek key lives only on the gateway; nothing is baked into the app. The gateway gives each person 100,000 tokens (input + output), one device or all of a person's devices once they sign in, and is the only counter: Settings shows what `GET /v1/trial` says. The trial is used only when nothing of the user's own works, only on the host, never in a container. Each call is signed with the instance's device key (`X-Nandi-Instance`, `X-Nandi-Key`, `X-Nandi-Time`, `X-Nandi-Signature` over `nandi-trial-v1\n<METHOD>\n<path>\n<instance>\n<time>\n<hex sha256(body)>`) and streamed through the DeepSeek adapter's parser with no bearer key.
- **Where:** `services/llm_trial/` (`mod.rs` modes and health, `gateway.rs` signed client), `llm/mod.rs` (`dispatch`), `llm/deepseek.rs` (`request_body`, `read_stream`), `services/llm_keys.rs` (`usable_model`, `get_free_trial_account`); Settings › Models & runtimes › Free tokens (`FreeTrialField`, `lib/free-trial.ts`); HTTP `GET /api/free-trial/account`; onboarding model step.
- **Preconditions:** No own keys; internet (or `NANDI_TRIAL_GATEWAY_URL` pointing at a local gateway).
- **Steps:** 1. Fresh install, ask something. 2. Open Settings › Models & runtimes › Free tokens. 3. Ask again and reopen the card.
- **Expected:** The answer streams. The card says "In use: N of 100,000 free tokens left." and N drops after each call (the gateway's count). Used up: "Your free tokens are used up. ..." and the switch disabled.
- **Edge / negative cases:** Offline: "Can't reach the free trial right now. ..." on the card, and the Ask box answers from the help guide (SH-39). Gateway paused (daily cap): "paused for today" with when it's back. Reinstalling or deleting `~/.nandi` doesn't reset anything: the gateway counts per device key and per person.
- **Automation:** Covered by: `llm_trial/tests.rs`, `llm_trial/gateway_tests.rs` (fake gateway: streaming, `/v1/trial`, golden signing string, end-to-end through `llm::complete`), `free-trial.test.ts`, `LlmKeysSection.test.tsx`, `offline_suite::with_the_trial_gateway_unreachable_the_ask_box_still_helps` | Layer: unit | Gap: the live gateway is manual.
- **Needs relay:** no
- **Status:** Shipped

### MD-14 — Free tokens switch and automatic turn-off

- **What:** The "Use free tokens" switch turns the trial on (remembering the current default, making the trial model default) or off (giving the previous default back unless the user picked another since). The trial turns itself off for good after a call on the user's own model succeeds (key, working Claude Code sign-in, Ollama default, llm.yaml/env model); saving a key also turns it off. It can be turned back on while the gateway hasn't said the tokens are used up.
- **Where:** `FreeTrialField` switch `#free-trial-switch`, `commands/llm_keys.rs` (`set_free_trial`), `llm_trial/mod.rs` (`set_enabled`, `own_model_worked`, `own_model_set_up`, `remember_default`, `take_remembered_default`). HTTP `PUT /api/free-trial`. Keychain `nandi.trial.deepseek.mode`, `nandi.trial.deepseek.previous-model`.
- **Preconditions:** Tokens left on the gateway.
- **Steps:** 1. Turn on with default `claude:x`. 2. Turn off. 3. Turn on, change default to `gpt:y`, turn off. 4. Add a working key and make a call.
- **Expected:** 1. Default becomes `deepseek:deepseek-flash`. 2. Back to `claude:x`. 3. Stays `gpt:y`. 4. Message "Off, because you set up your own model. You still have N free tokens."; while on, `deepseek:` calls use the trial before the user's DeepSeek key.
- **Edge / negative cases:** reading status never writes the keychain; a stale Claude Code login doesn't turn it off; turning on once the gateway said used up is refused; an unreadable mode counts as off.
- **Automation:** Covered by: `llm_trial/tests.rs`, `llm_keys.rs` tests, `LlmKeysSection.test.tsx` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-15 — Trial errors, Retry-After and the gateway's state

- **What:** Gateway error codes become clear messages: `trial_used_up` (add your own model), `trial_not_enough` (tokens left, shorter conversation), `trial_busy`, `trial_paused` (back in about N hours), `rate_limited` (try again in N seconds), `too_many_devices`, `stale_time` (fix the clock), `key_mismatch`, `upstream_busy`, `upstream_error`. The trial path doesn't use the DeepSeek adapter's retries: a short `Retry-After` (10 s or less) on `trial_busy`, `rate_limited` or `upstream_busy` is waited out once; longer ones are reported. `stale_time` adjusts the clock by the server's `serverTime` and signs again once. Signed times always go up (no replays). Used up, paused (until `Retry-After`) and unreachable (for a minute) are remembered in memory, so calls stop offering the trial and a computer with no model gets the help guide at once.
- **Where:** `llm_trial/gateway.rs` (`GatewayError::user_message`, `chat`, `send`), `llm_trial/mod.rs` (`Trial::note`, `why_unavailable`), `LlmKeyNotice.tsx` (paused/unreachable lines).
- **Preconditions:** A local gateway (`NANDI_TRIAL_GATEWAY_URL`) whose limits can be set low.
- **Steps:** 1. Set `NANDI_TRIAL_DEVICE_RPM=1` on the gateway and ask twice quickly. 2. Set `NANDI_TRIAL_DAILY_CAP=1` and ask. 3. Move this computer's clock 10 minutes off and ask. 4. Stop the gateway and ask.
- **Expected:** 1. "Too many free-trial calls from this computer. Try again in N seconds." 2. "The free trial is paused for today ..." and the notice "The free trial is paused for today." 3. The answer arrives (clock corrected once). 4. A help-guide answer; Settings says it can't reach the trial.
- **Edge / negative cases:** a captive-portal page instead of JSON reads "HTTP <status>: ..."; a second busy answer isn't retried again.
- **Automation:** Covered by: `gateway_tests.rs` (`every_error_code_has_a_clear_message`, `a_short_retry_after_is_waited_out_once`, `a_long_retry_after_is_reported_not_waited`, `a_stale_clock_is_corrected_by_the_servers_and_signed_again`, `paused_and_used_up_are_never_retried`, `signed_times_only_go_up`), `llm_trial/tests.rs` (`paused_and_unreachable_are_not_offered_for_a_while`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-16 — No-model notice banner

- **Duplicate:** tested under SH-20. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### MD-17 — Onboarding model step

- **Duplicate:** tested under ON-05, ON-06, ON-07, ON-08. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### MD-18 — Team LLM config (`.nandi/llm.yaml`) and Enterprise config panel

- **What:** A team repo can carry `.nandi/llm.yaml` (providers bedrock/azure/litellm/anthropic/openai/ollama, routing default/fallbacks, no secrets); precedence env (`NANDI_LLM_PROVIDER`, `NANDI_LLM_MODEL`) > repo > local; harness agents take its provider/model, native agents its native default model. Advanced shows what's in effect, with Sync.
- **Where:** Settings → Models & runtimes → Advanced → Enterprise config. `src/components/preferences/EnterpriseConfigSection.tsx`, `services/config/repo_config.rs`, `commands/config.rs` (`get_repo_llm_config`, `reload_repo_llm_config`). HTTP `GET /api/config/llm`, `POST /api/config/llm/reload`. Event `config://repo_llm_changed`.
- **Preconditions:** A repo with `.nandi/llm.yaml`.
- **Steps:** 1. Add the file with `default: bedrock-primary`. 2. Click Sync. 3. Break the YAML, Sync. 4. Set `NANDI_LLM_MODEL=ollama:llama3`, restart.
- **Expected:** 2. Toast "Enterprise config synced", effective values shown, `repo_config_cache` row. 3. Toast "The repo's .nandi/llm.yaml has a problem". 4. Default model for calls is `ollama:llama3`. File changes on disk are picked up by mtime without Sync.
- **Edge / negative cases:** invalid env model ignored; `NANDI_LLM_*` names are not read.
- **Automation:** Covered by: `repo_config.rs` tests, `src/components/preferences/EnterpriseConfigSection.test.tsx` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-19 — AWS sign-in for Bedrock (profiles, SSO login, check)

- **What:** Shows the AWS CLI version (needed for SSO), the profiles from the AWS config with their kind (SSO, access keys, assumed role, credential process, default chain), a Check that resolves credentials (account, ARN with show/hide), "Log in with SSO" in a dialog with the device code and a link, and whether a Bedrock API key is set (environment or Claude Code settings).
- **Where:** Settings → Cloud credentials → AWS → "Sign-in for Bedrock models". `src/components/harness/AwsAuthPanel.tsx`, `SsoLoginDialog.tsx`, `aws-identity.ts`, `commands/aws_auth.rs` (`detect_aws_cli`, `list_aws_profiles`, `get_aws_profile`, `start_aws_sso_login`, `poll_aws_sso_login`, `check_aws_credentials`, `get_aws_bedrock_api_key_status`), `services/harness/aws_auth.rs`, `aws_config_parser.rs`. HTTP `/api/aws/cli`, `/api/aws/profiles`, `/api/aws/profiles/{name}`, `POST /api/aws/sso/login`, `GET /api/aws/sso/login/{id}`, `POST /api/aws/credentials/check`, `GET /api/aws/bedrock-api-key`. Events `aws://profile_changed`, `aws://credentials_expired`.
- **Preconditions:** AWS CLI v2, an SSO profile.
- **Steps:** 1. Pick the profile, Check. 2. Let the session expire, Check. 3. Log in with SSO, approve in the browser.
- **Expected:** "Works" with account; "Your SSO session has expired: log in again."; dialog "Waiting for you to approve in the browser…" then "Logged in.".
- **Edge / negative cases:** no AWS CLI ("AWS CLI not installed (needed for SSO login)"); no profiles ("No AWS profiles in your AWS config. Run aws configure sso to add one."); Cancel during login.
- **Automation:** Covered by: `src/components/harness/aws-auth.test.tsx`, `aws-identity.test.ts`, `aws_auth.rs`, `aws_config_parser.rs` tests | Layer: manual (real AWS) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-20 — Bedrock model picker

- **What:** Lists Bedrock models from a bundled registry (Anthropic native in Claude Code vs third-party through the gateway, tested/untested/known issues/unsupported badges), filterable, refreshable from Bedrock; picking one says how Claude Code reaches it; "Needs AWS sign-in:" with a Cloud credentials link.
- **Where:** Settings → Models & runtimes → Coding agents → "Bedrock models". `src/components/harness/BedrockModelSelector.tsx`, `RuntimesSection.tsx` (`BedrockGateway`), `commands/bedrock.rs` (`list_bedrock_models`, `refresh_bedrock_models`), `services/harness/bedrock_models.rs`, `bedrock_model_registry.json`. HTTP `GET /api/bedrock/models`, `POST /api/bedrock/models/refresh`. Event `bedrock://models_refreshed`.
- **Preconditions:** AWS sign-in for refresh.
- **Steps:** 1. Open the selector, filter "llama". 2. Refresh.
- **Expected:** Grouped list with badges; refresh updates `bedrock_models_cache`.
- **Edge / negative cases:** not signed in (refresh error toast); the pick isn't saved anywhere (only drives the gateway start button).
- **Automation:** Covered by: `bedrock_models.rs` tests, `harness-ui.test.tsx` | Layer: api | Gap: selection not persisted.
- **Needs relay:** no
- **Status:** Partial (chosen model is not saved as a runtime default)

### MD-21 — LiteLLM gateway for third-party Bedrock models

- **What:** For a third-party Bedrock model, "Start gateway" runs `ghcr.io/berriai/litellm:v1.102.0` as `nandi-litellm` on Docker network `nandi-harness` (its way to Bedrock) and, since #9, also on the internal `nandi-agents` (so coding agents reach it directly, and it is in their `NO_PROXY`), published on `127.0.0.1:4000` for its health check, with aliases `nandi-bedrock` / `nandi-bedrock-small`, AWS credentials and a per-start master key in its env (config file has no secrets); Claude Code gets `ANTHROPIC_BASE_URL=http://nandi-litellm:4000`. Status shows running/stopped and "Serving <model> at <endpoint>", with Stop.
- **Where:** Bedrock models block → Start gateway / `LitellmGatewayStatus.tsx`; `commands/bedrock.rs` (`start_litellm_gateway`, `stop_litellm_gateway`, `get_litellm_gateway_status`), `services/harness/gateway.rs`, `runtimes/nandi-harness/litellm-config-template.yaml`. HTTP `GET /api/bedrock/gateway`, `POST /api/bedrock/gateway/start`, `POST /api/bedrock/gateway/stop`. Event `bedrock://gateway_status`.
- **Preconditions:** Docker, AWS sign-in, image pullable.
- **Steps:** 1. Pick a Llama model, Start gateway. 2. `docker ps`. 3. Run a `runtime: claude-code` agent with `runtime_config.gateway: litellm`. 4. Stop.
- **Expected:** Ready within 90 s; container on `127.0.0.1:4000`; `docker inspect nandi-litellm` lists both `nandi-harness` and `nandi-agents` (checklist 4.10); agent run goes through it with no model hosts opened on the proxy (SB-15); Stop removes it.
- **Edge / negative cases:** invalid model id rejected (`valid_model_id`); readiness timeout; agent needing a different model restarts it; attaching to `nandi-agents` fails ("Could not attach the gateway to the agents' network", container removed).
- **Automation:** Covered by: `gateway.rs` tests | Layer: manual | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### MD-22 — Free tokens linked to the connected email (no sign-in)

- **What:** The free tokens count per computer (all Nandis on it share them). There is no sign-in button: once Outlook or Gmail is connected, the trial links itself to that email (in the background after connecting, and whenever the card finds the computer unlinked), and the card says "Linked to <email>: your free tokens are shared by all your computers."; until then "These free tokens count for this computer. Connect Outlook or Gmail in Settings › Email to share them across your computers." Linking sends a fresh ID token of the identity's mailbox to the gateway (`POST /v1/link`); every call carries the machine hash (the trial's salt), signed. The computer's earlier usage counts toward the person.
- **Where:** `LlmKeysSection.tsx` (`TrialLink`), `lib/free-trial.ts` (`trialLinkLine`), `llm_trial/mod.rs` (`account`, `link`, `link_soon`), `llm_trial/gateway.rs` (`link`, the `X-Nandi-Machine` header, `nandi-trial-v2`), `email/mod.rs` (`connect` calls `link_soon`).
- **Preconditions:** A gateway with `NANDI_TRIAL_GOOGLE_CLIENT_ID` / `NANDI_TRIAL_MICROSOFT_CLIENT_ID` set to the build's mail client IDs.
- **Steps:** 1. Fresh home, no email: open the card. 2. Connect Gmail in Settings › Email, reopen the card. 3. On a second computer, connect the same email.
- **Expected:** 1. The per-computer line, no sign-in buttons. 2. "Linked to <email>". 3. Both cards show the same used/limit.
- **Edge / negative cases:** a computer already linked to another email shows "This computer's free trial is linked to another email…"; a Nandi copied to another computer gets the machine-mismatch sentence; a mailbox that can't give an ID token leaves the trial working per computer.
- **Automation:** Covered by: `gateway_tests.rs` (`linking_sends_the_id_token`, `the_signing_string_matches_the_gateway`, `every_error_code_has_a_clear_message`), `LlmKeysSection.test.tsx` ('says the tokens count per computer…', 'shows the linked email'), `free-trial.test.ts`, trial-gateway `tests/gateway.rs` | Layer: unit | Gap: the live gateway with real ID tokens is manual.
- **Needs relay:** no
- **Status:** Shipped

## Sandbox security and egress

### SB-01 — Container hardening (tool and coding-agent containers)

- **What:** Both container kinds run with read-only root filesystem, tmpfs for `/tmp` and the home dir (owned by the container's user), the agent user set by the app (`runtime::AgentUser`: the image's `1000:1000` on Docker Desktop, the host user's own uid and gid on a Linux engine, never root, with `HOME` and `USER` set), `cap_drop ALL`, `no-new-privileges`, pids limit (256 native, 512 harness), Docker's default seccomp; harness containers use `--rm --init`, native ones `init: true` (since #9). Network isolation is SB-13.
- **Where:** `src-tauri/src/runtime.rs` (`AgentUser`, `container_config`), `services/harness/adapter.rs`; Dockerfiles label `org.nandi.sandbox="6"`. Stdio connector containers get the same hardening (CP-22).
- **Preconditions:** Docker, both images built (`sandbox=5`).
- **Steps:** 1. Start a native agent container and a harness run. 2. `docker inspect` both. 3. From a tool call, `touch /usr/x`; `id`. (Same as `docs/developer/sandbox-test-checklist.md` 1.2–1.5.)
- **Expected:** `ReadonlyRootfs: true`, `User: 1000:1000` on Docker Desktop (`<your uid>:<your gid>` on a Linux engine), `CapDrop: [ALL]`, `SecurityOpt: [no-new-privileges]`, `PidsLimit` set, tmpfs mounts; write to `/usr` fails; the workdir is writable.
- **Edge / negative cases:** older image refused (RT-03); pip/npm caches only in tmpfs; on a Linux engine a host user other than uid 1000 (GitHub's runner is 1001) can write the workdir and its writable shared folders, nothing is made world-writable; Nandi run as root there keeps uid 1000.
- **Automation:** Covered by: `runtime.rs` tests, `adapter.rs` tests; `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images): `the_native_container_is_locked_down` and `the_coding_agent_container_is_locked_down` inspect both kinds (read-only root, `CapDrop [ALL]`, `no-new-privileges`, pids 256 / 512, `Init`, the agent user and `id -u` inside, the tmpfs mounts, the workdir writable with no folder opened up) and plant host credentials (Claude Code's `.credentials.json` refresh token, `~/.aws/credentials`, an SSO cache, a `pip.conf` and an `.npmrc` token, provider and AWS keys in the keychain, host env tokens) around the agent's folders and in the coding agent's detected home: none is in the env or in any file inside (`grep -r` over `/`) | Layer: api (Docker) | Gap: none. The short-lived Claude access token a coding-agent container may get is by design (see Gaps and planned).
- **Needs relay:** no
- **Status:** Shipped

### SB-02 — Ask before CLI tools act (harness approvals)

- **What:** For Claude Code, Codex and Gemini CLI, tool kinds in the agent's `requires_approval` (default `shell` and `write_file`) wait on an Inbox approval (rows marked `harness:`); the decision reaches the waiting CLI call directly; no decision within an hour rejects it; a run that ends stops waiting; after a restart nothing waits, so deciding such an approval cancels its run.
- **Where:** Inbox approval card (other area) for the request; `services/harness/approvals.rs` (`decide`, `is_waiting`, `close_stale`, `CALL_PREFIX`), `services/harness/acp.rs`, `src/components/inbox/approval-model.ts`. Event `harness://permission_request`.
- **Preconditions:** Docker, harness image, a working CLI sign-in; agent with default `requires_approval`.
- **Steps:** 1. Ask a claude-code agent to run `ls`. 2. Approve in the Inbox. 3. Repeat and Reject. 4. Repeat, restart the app before deciding, then decide.
- **Expected:** 1. Run pauses with an Inbox approval. 2. Command runs, run continues. 3. CLI gets a rejection and the run continues/ends accordingly. 4. Deciding cancels the run (`close_stale`).
- **Edge / negative cases:** one-hour TTL (`APPROVAL_TTL_MS`); run cancelled while waiting; `requires_approval: []` (no prompts).
- **Automation:** Covered by: `approvals.rs` tests, `acp.rs` tests, `src/components/inbox/approval-model.test.ts`, `src/components/views/RunDetailView.test.tsx` | Layer: manual (real CLI) + unit | Gap: no end-to-end test with a CLI.
- **Needs relay:** no
- **Status:** Shipped

### SB-03 — Egress proxy server and per-container tokens

- **What:** One host server on `127.0.0.1:11435` (another free port if taken) is the only way out for every agent container, native and coding agents; since #9 containers reach it as `nandi-egress:11435` through the relay (SB-14), which forwards without deciding anything. Every route needs that container's proxy token; a body `agent_id` naming another agent is refused 403; capabilities are re-read per request: the agent's skill.md narrowed by its policies (`RepoStore::skill`, PO-13), or for a stdio connector's container (`connector:<id>`) its package registries and `config.network` (CP-22).
- **Where:** `src-tauri/src/services/egress/mod.rs` (`start_server`, `proxy_port`, `container_env`), `src-tauri/src/runtime.rs` (`ProxyTokens::issue/revoke/agent_for`). Docs `docs/developer/egress.md`.
- **Preconditions:** App running.
- **Steps:** 1. `curl http://127.0.0.1:11435/fetch` with no token. 2. With a token and another agent's id.
- **Expected:** 401; 403.
- **Edge / negative cases:** port taken by a second instance (proxy binds elsewhere; the relay is recreated for the new port, label `org.nandi.egress.target`, checklist 4.11); token revoked on container stop; coding agents get a token too (`ProxyTokens::issue` in `runtime_router::resolve_runtime`).
- **Automation:** Covered by: `services/egress/mod.rs` tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-04 — LLM proxy for containers (`POST /llm`)

- **What:** Containers ask for completions with `NANDI_LLM_PROXY_TOKEN`; the host injects the provider key, so keys never enter native containers; the agent in `llm.log` comes from the token (`source: proxy`); body limit 8 MB.
- **Where:** `services/egress/llm_proxy.rs`.
- **Preconditions:** Running native container; a key.
- **Steps:** 1. From inside the container, `curl -H "Authorization: Bearer $NANDI_LLM_PROXY_TOKEN" -d '<LlmRequest>' $NANDI_LLM_PROXY_URL`.
- **Expected:** `LlmResponse` JSON; `llm.log` line with the agent id.
- **Edge / negative cases:** bad token 401; oversized body 413.
- **Automation:** Covered by: `llm_proxy.rs` tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-05 — Allow-listed HTTP fetch (`POST /fetch`)

- **What:** HTTP requests to hosts in the agent's `network.allow` (optional `methods`, wildcard subdomains, non-default ports); anything else denied; redirects are returned, not followed.
- **Where:** `services/egress/http_proxy.rs`, `services/egress/capabilities.rs`.
- **Preconditions:** Agent with `network.allow: [{host: api.github.com, methods: [GET]}]`.
- **Steps:** 1. GET api.github.com via `$NANDI_FETCH_PROXY`. 2. POST it. 3. GET example.com. 4. Edit skill.md to allow example.com, retry without restart.
- **Expected:** 1 allowed; 2, 3 denied (403); 4 allowed immediately. All entries in `egress.log`.
- **Edge / negative cases:** `*.example.com` matches subdomains only; 3xx returned to caller.
- **Automation:** Covered by: `http_proxy.rs`, `capabilities.rs` tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-06 — HTTPS tunnel (`CONNECT` via `HTTPS_PROXY`)

- **What:** Tools honoring `HTTPS_PROXY` tunnel through the relay and the proxy (Basic auth password = token) to allowed hosts on 443 (or the rule's port); hosts whose rule limits methods are refused (methods not visible in TLS); `sts` agents can also tunnel to `*.amazonaws.com`; a coding agent's model hosts are added for its run (SB-15). Since #9 the tunnel is enforced, not advisory: the container has no other route out (SB-13).
- **Where:** `services/egress/tunnel.rs`, `services/egress/mod.rs` (`start_server` sends `CONNECT` to `tunnel::handle_connect`).
- **Preconditions:** Agent with an unrestricted `network.allow` host.
- **Steps:** Follow `docs/developer/sandbox-test-checklist.md` 4.3 (the images have no `curl`: use the Python `urllib` line) and 4.4: 1. Fetch an allowed host. 2. Fetch an unlisted host. 3. Fetch a method-limited host. 4. Repeat 1 with the proxy variables unset (`env -u HTTPS_PROXY -u https_proxy -u HTTP_PROXY -u http_proxy`).
- **Expected:** 1 works; 2 and 3 refused by the proxy; 1–3 in `egress.log` (allowed / denied); 4 fails with no route.
- **Edge / negative cases:** `HTTP_PROXY` points at the same address, but the proxy serves only `CONNECT` and its `/llm`, `/aws`, `/fetch` routes, so a plain `http://` URL through it doesn't reach the site (use `$NANDI_FETCH_PROXY`); `git` honors `HTTPS_PROXY` and is refused for unlisted hosts (checklist 4.7).
- **Automation:** Covered by: `tunnel.rs` tests (real CONNECT through the server); `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images): from inside both container kinds, a CONNECT with the container's own `HTTPS_PROXY` to an allowed host gets `200` and one to a host outside `network.allow` gets `403` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped (was Partial until #9 `d9af777`: containers were on the default bridge network)

### SB-07 — AWS proxy and scoped STS sessions

- **What:** `POST /aws/{service}/{action}` runs AWS calls (MVP: S3 GetObject/PutObject/ListObjects) under a GetFederationToken session limited to the agent's `aws` grants (1 h, cached per agent+policy hash until 60 s before expiry); `aws.mode: sts` instead injects 12 h scoped credentials as `AWS_*` env at container start (start fails without an AWS key).
- **Where:** `services/egress/aws_proxy.rs`, `credentials.rs` (`CredentialsManager::assume_role`), `capabilities.rs` (`session_policy`).
- **Preconditions:** AWS access key set (SB-08), agent with `aws` grants.
- **Steps:** 1. Get an allowed object. 2. Get one outside `resources`. 3. Start an `sts` agent with no key.
- **Expected:** 1 ok; 2 denied by the proxy (and by AWS); 3 start error.
- **Edge / negative cases:** `ListObjects` needs the bucket ARN; changing key/region clears the cache.
- **Automation:** Covered by: `aws_proxy.rs`, `credentials.rs`, `capabilities.rs` tests | Layer: api (injected executors) + manual (real AWS) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-08 — AWS access key and region for agents' AWS calls

- **What:** A write-only "Access key ID" + "Secret access key" and "Region", used for agents' AWS calls through Nandi; shown redacted (`AKIA…WXYZ`); removable with confirmation ("Remove the AWS access key?", Remove key).
- **Where:** Settings → Cloud credentials → AWS → "Access key for agents" (`data-settings-focus="aws-access-key"`). `CloudCredentialsSection.tsx` (`AccessKeyForm`), `commands/egress.rs` (`get_egress_status`, `set_aws_credentials`, `set_aws_region`). HTTP `GET /api/egress/status`, `PUT /api/egress/aws-credentials`, `PUT /api/egress/aws-region`. Keychain `nandi.aws.access_key`, `nandi.aws.secret_key`, `nandi.aws.region`.
- **Preconditions:** An IAM key.
- **Steps:** 1. Enter key and region, click Save credentials. 2. Reload. 3. Click Remove, then Remove key.
- **Expected:** Toast "AWS credentials saved"; status shows redacted id and region; secret never returned; remove asks first, then "AWS credentials removed".
- **Edge / negative cases:** status load error ("Couldn’t read the AWS settings."); `Debug` output redacted.
- **Automation:** Covered by: `credentials.rs`, `egress/mod.rs` tests | Layer: api | Gap: no UI test for `CloudCredentialsSection`.
- **Needs relay:** no
- **Status:** Shipped

### SB-16 — Each agent's AWS identity: its own profile, the access key, or the sign-in

- **What:** An agent's AWS calls run as, in order:
  1. The AWS profile chosen for it: agent page → Settings → AWS → "AWS profile", saved as skill.md `aws.profile`.
  2. The access key in Settings → Cloud credentials.
  3. The sign-in profile there (`AWS_PROFILE`, else the SDK's default chain).

  Long-term keys (the access key, or a profile with static keys) are narrowed to the agent's grants with STS GetFederationToken. Temporary keys (SSO, assume-role, credential_process) can't be narrowed, so they serve only the `/aws` proxy, which checks every call against the grants. An `sts`-mode agent with only temporary keys doesn't start, and says why. Sessions are cached per agent, grants and profile.

- **Where:** `services/egress/credentials.rs` (`CredentialsManager::session`, `SessionUse`, `SignInKeys`), `harness/aws_auth.rs` (`agent_keys`, `agent_sign_in`), `aws_proxy.rs`, `egress/mod.rs` (`container_env`), `repo_service.rs` (`aws_profile`), `src/components/agents/AwsProfileField.tsx`, `AgentSettingsTab.tsx`.
- **Preconditions:** Two AWS profiles (one SSO, one with static keys), an agent with an `aws:` section granting `s3:ListObjects` on a bucket.
- **Steps:**
  1. Remove the access key. Sign in with the SSO profile on Cloud credentials. Ask the agent to list the bucket.
  2. On the agent's Settings, choose the static-keys profile; save; ask again.
  3. Set `aws.mode: sts` and choose the SSO profile; start the agent.
  4. Clear the profile and add an access key; ask again.
- **Expected:**
  - Step 1: listed with the sign-in.
  - Step 2: listed as the chosen profile; skill.md has `aws: profile:` and its grants unchanged.
  - Step 3: refused with "AWS profile '…' gives temporary sign-in keys…, use proxy mode, or add an access key".
  - Step 4: listed with the key.
  - The AWS section shows only for agents with an `aws:` section or a profile. "Default: the access key, else the sign-in" clears it. A saved profile missing from the AWS config shows "(not in your AWS config)".
- **Edge / negative cases:**
  - An expired SSO sign-in says to log in again from Settings → Cloud credentials (`aws sso login --profile …`).
  - A profile name with spaces is refused on save.
  - Clearing the profile of an agent with no grants removes its `aws:` section.
- **Automation:** Covered by: `credentials.rs` tests (`without_a_key_agents_use_the_sign_in`, `an_agent_s_own_profile_wins_over_the_key`), `repo_service.rs` (`the_aws_profile_is_set_and_cleared_without_touching_the_grants`), `useAgentEditor.test.ts`, `AwsProfileField.test.tsx` | Layer: unit | Gap: live AWS (`manual`).
- **Needs relay:** no
- **Status:** Shipped

### SB-17 — Log in and set up AWS profiles in Cloud credentials

- **What:** Settings → Cloud credentials → Sign-in offers what each profile needs: **Log in** for SSO (`aws sso login`) and for `login_session` profiles where the CLI has `aws login`; **Replace keys** for an access-key profile. **Add access keys** writes a new profile with `aws configure` (secret on stdin). **Add SSO profile** takes the start URL and SSO region, opens the IAM Identity Center sign-in with the code, then lists the accounts and roles to pick from, writes the profile with `aws configure set`, and runs `aws sso login` for it. Nandi writes no AWS file itself.
- **Where:** `harness/aws_auth/setup.rs` (`save_key_profile`, `start_sso_setup`, `poll_sso_setup`, `finish_sso_setup`), `aws_auth.rs` (`start_sso_login`, `detect_cli` `supportsLogin`), `aws_config_parser.rs` (`ConsoleLogin`); commands `save_aws_key_profile`, `start_aws_sso_setup`, `poll_aws_sso_setup`, `finish_aws_sso_setup`, `cancel_aws_sso_setup`; HTTP `POST /api/aws/profiles/keys`, `POST /api/aws/sso/setup`, `GET|DELETE /api/aws/sso/setup/{id}`, `POST /api/aws/sso/setup/finish`; `AwsAuthPanel.tsx`, `AwsKeysDialog.tsx`, `AwsSsoSetupDialog.tsx`, `aws-setup-model.ts`.
- **Preconditions:** The AWS CLI v2. For SSO, an IAM Identity Center start URL you can sign in to.
- **Steps:**
  1. Add access keys: name `build`, a key ID and secret, region `eu-west-1`.
  2. Pick `build`; Replace keys with new ones, region blank.
  3. Add SSO profile: your start URL and SSO region; approve in the browser; pick an account and role; Save and log in; approve the CLI's login.
  4. Pick the new profile and Check.
- **Expected:**
  - Step 1: the profile appears and is selected; the credentials file has it; the secret is never shown again.
  - Step 2: the keys change and the region stays.
  - Step 3: the code shows, the accounts and roles list, the name is suggested (`prod-readonly`), and the login dialog says Logged in.
  - Step 4: Works, with the account.
- **Edge / negative cases:** a name with spaces, a bad region, or an `http://` start URL is refused before saving; a denied or expired sign-in says so with Try again; closing the dialog drops the setup; an account or role not in the list is refused by the backend; no AWS CLI disables both Add buttons with "Install the AWS CLI v2"; a console profile on a CLI without `aws login` says to update it.
- **Automation:** Covered by: `aws_auth/setup.rs` tests (rules, stdin answers, settings, the device sign-in against a fake directory, denied, the real CLI writing both kinds in an ignored test), `aws_config_parser.rs` (console profile), `aws-auth.test.tsx` (add keys, the SSO flow, Log in per kind), `aws-setup-model.test.ts` | Layer: unit | Gap: a live IAM Identity Center sign-in (`manual`).
- **Needs relay:** no
- **Status:** Shipped

### SB-09 — Egress audit log (`egress.log`)

- **What:** Every `aws`, `fetch` and `tunnel` request, allowed or denied, is appended as JSON to `~/.nandi/local/logs/egress.log`, with URL query strings and userinfo dropped.
- **Where:** `services/egress/mod.rs`, `src-tauri/src/audit.rs` (`AuditLog::Egress`).
- **Preconditions:** Egress traffic.
- **Steps:** 1. Make allowed and denied requests with `?token=x` in the URL. 2. Read the file.
- **Expected:** Entries with agent, route, host, decision; no query strings or secrets.
- **Edge / negative cases:** since #9 coding agents' tunnels are logged too, including their model hosts (`api.anthropic.com` allowed for that agent, checklist 4.6).
- **Automation:** Covered by: `egress/mod.rs` tests | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-10 — Internet access settings page

- **What:** "Allowed sites by agent" lists each agent's allowed sites ("No internet access" when none, "Any site" for `*`) with Edit (opens the agent's Settings, where they're changed: AG-29), an Agent filter ("All agents"), "Recent" egress requests with relative times and "See all in Logs", a warning when the proxy is down, and the proxy details under Advanced. A blocked fetch or tunnel by an agent made here has **Allow**, which adds that site (with its port unless 443) to the agent's skill.md and toasts "{agent} can now reach {site}".
- **Where:** Settings → Internet access. `src/components/settings/sections/InternetAccessSection.tsx`, `src/hooks/useEgressStatus.ts`, `useAllowSite` (`useAgentEditor.ts`), `get_egress_status` (`EgressStatus.recentEntries`, `routes`, `proxyUrl`).
- **Preconditions:** Agents, some egress traffic, one blocked request.
- **Steps:** 1. Open the page. 2. Filter by one agent. 3. Click Edit. 4. Click Allow on a blocked request, then have the agent retry it.
- **Expected:** Rows per agent; filter updates URL params (`agentId`); "No internet requests yet." when empty; "Agents can’t reach the internet right now." when the proxy isn't serving. Step 4: the site shows in that agent's row and the retry is allowed.
- **Edge / negative cases:** no agents ("No agents yet." with a "Create an agent" link); no Allow on allowed requests, AWS calls, or installed agents; Allow on an agent that already allows any site changes nothing.
- **Automation:** Covered by: `InternetAccessSection.test.tsx` (Any site, Allow saves the site, none for installed agents), `network-access-model.test.ts` (`blockedSite`) | Layer: unit + ui-e2e | Gap: step 4's retry is manual.
- **Needs relay:** no
- **Status:** Shipped

### SB-11 — Egress environment injected into agent containers

- **What:** Native and (since #9) coding-agent containers get `NANDI_LLM_PROXY`, `NANDI_LLM_PROXY_URL` (`http://nandi-egress:11435/llm`), `NANDI_LLM_PROXY_TOKEN`, `NANDI_AWS_PROXY`, `NANDI_FETCH_PROXY`, `HTTPS_PROXY`/`https_proxy` and `HTTP_PROXY`/`http_proxy` (`http://nandi:<token>@nandi-egress:11435`), `NO_PROXY`/`no_proxy` (`nandi-egress,nandi-litellm,localhost,127.0.0.1`), and `AWS_*` for `sts` native agents. Only `NANDI_*` names.
- **Where:** `services/egress/mod.rs` (`proxy_env`, `container_env`), `runtime.rs` (`start_container`), `services/orchestrator/runtime_router.rs` (`plan`, for harness containers).
- **Preconditions:** Running native container.
- **Steps:** 1. Run `env` via a native agent's shell tool. 2. Ask a claude-code agent to run `env`.
- **Expected:** Variables present in both, each with its own token; no provider API keys in native containers (coding-agent containers still carry their sign-in, see Gaps and planned).
- **Edge / negative cases:** the harness Test connection container (RT-10) gets none of these.
- **Automation:** Covered by: `egress/mod.rs` (`proxy_env_points_at_the_relay`), `runtime.rs` config test, `runtime_router.rs` (`coding_agents_leave_only_through_the_proxy_to_their_model`), `sandbox_suite.rs` (env has `nandi-egress:11435`, no `API_KEY`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-12 — Secrets in the OS keyring

- **Duplicate:** tested under DT-04. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### SB-13 — Agents' internal network (`nandi-agents`)

- **What:** Every agent container (native tool containers and coding-agent containers) joins only `nandi-agents`, a Docker bridge network created `--internal` (label `nandi.agents=1`): no route to the internet or the host. Its one way out is the relay (SB-14) to the egress proxy. A process that ignores `HTTPS_PROXY` gets nowhere. If a network of that name exists but isn't internal, it is refused, not used, and no agent starts.
- **Where:** `src-tauri/src/services/egress/relay.rs` (`AGENT_NETWORK`, `ensure_ready`, `ensure_network`, `ensure_agent_network`), `src-tauri/src/runtime.rs` (`container_config` `network_mode`), `services/orchestrator/runtime_router.rs` (`plan`: `Placement::Container { network: nandi-agents }`), `services/harness/adapter.rs` (`--network=`). Docs `docs/developer/egress.md` ("The agents' network").
- **Preconditions:** Docker, both images rebuilt at `org.nandi.sandbox=5`.
- **Steps:** Follow `docs/developer/sandbox-test-checklist.md` 4.2, 4.4 and 4.12: 1. Run any agent, then `docker network inspect nandi-agents --format "{{.Internal}}"`. 2. From a native agent's shell, fetch an allowed host with the proxy variables unset. 3. Quit, `docker network rm nandi-agents`, `docker network create nandi-agents` (not internal), run an agent.
- **Expected:** 1. `true`. 2. Fails with no route. 3. The run fails with "The Docker network nandi-agents exists but isn't internal. Remove it (docker network rm nandi-agents) and Nandi recreates it."; no container starts.
- **Edge / negative cases:** network created concurrently by two starts (second create error is ignored when the network now exists); the harness Test connection container is not on it (RT-10); the LiteLLM gateway also joins it (MD-21).
- **Automation:** Covered by: `runtime.rs` config test (`network_mode` `nandi-agents`), `runtime_router.rs` tests (network is `nandi-agents` with and without a gateway); `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images): `the_agents_network_is_internal_or_refused` (the real network is internal; a same-named network that isn't is refused with the message above; a missing one is created internal with `nandi.agents=1`), and from inside both container kinds no connection to the internet (`1.1.1.1:443`), to the host's proxy port directly or to `host.docker.internal` | Layer: api (Docker) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### SB-14 — Egress relay container (`nandi-egress`)

- **What:** Before an agent container starts, Nandi makes sure the `nandi-egress` relay is running. It runs from the image that is starting the agent (`python /opt/nandi/relay.py` in `nandi-runtime`, `node /opt/nandi/relay.mjs` in `nandi-harness`), sits on the default bridge and on `nandi-agents`, listens on `11435`, and forwards every connection to `host.docker.internal:<proxy port>`. On Docker Desktop that is the host's loopback, where the proxy listens; on a Linux engine (`host-gateway`) it is the default bridge's gateway, so before the relay starts the proxy also listens on that one address (from `docker network inspect bridge`; never on other interfaces; the token check stays the guard; an address that isn't the host's, as with Docker Desktop for Linux, is skipped). It holds no env, tokens or secrets and decides nothing: read-only root, `cap_drop ALL`, `no-new-privileges`, 64 MB, pids 64, `init`, user `1000:1000`. A relay that is stopped, off `nandi-agents`, or labelled for another proxy port (`org.nandi.egress.target`) is removed and recreated. If the relay can't be created or started, the agent doesn't start.
- **Where:** `src-tauri/src/services/egress/relay.rs` (`ensure_relay`, `reach_the_proxy`, `relay_config`, `RelayImage`, `RELAY_NAME`, `RELAY_PORT`, `TARGET_LABEL`), `egress/mod.rs` (`listen_on_bridge_gateway`), `runtimes/nandi-runtime/relay.py`, `runtimes/nandi-harness/relay.mjs`, `services/images/mod.rs` (build contexts include them).
- **Preconditions:** Docker, images at `org.nandi.sandbox=5`.
- **Steps:** 1. Run any agent; `docker ps --filter name=nandi-egress` (checklist 4.2). 2. `docker inspect nandi-egress` (networks, labels, `Config.Env`). 3. `docker stop nandi-egress`, run the agent again. 4. Start a second Nandi with another `NANDI_HOME` so its proxy takes another port, and run an agent (checklist 4.11).
- **Expected:** 1. Running. 2. On `bridge` and `nandi-agents`, label `org.nandi.egress.target=<proxy port>`, no env beyond the image's, `ReadonlyRootfs: true`. 3. Recreated and the agent reaches out again. 4. Relay recreated for the new port; the agent still reaches out. App log "Started the egress relay (nandi-egress → host port N)".
- **Edge / negative cases:** a relay left from an older run on a different port is replaced; attach or start failure removes the half-made relay and reports "Could not attach the egress relay" / "Could not start the egress relay"; two Nandi instances share the one relay name, and each agent start recreates it for its own proxy port, so after step 4 check whether the first instance's running agents still get out (their traffic now reaches the second proxy, which doesn't know their token).
- **Automation:** Covered by: `relay.rs` (`the_relay_forwards_to_the_host_proxy_and_holds_nothing`, `the_proxy_listens_on_the_bridge_s_ipv4_gateway`), `services/images/mod.rs` (`each_context_has_its_dockerfile_and_what_it_copies`); `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images): `the_egress_relay_holds_nothing_and_is_replaced_when_wrong` (hardening, 64 MB, pids 64, on `bridge` and `nandi-agents`, target label, no env beyond its image's; replaced when stopped, off `nandi-agents` or labelled for another port, untouched when right), and both relays carry traffic (`relay.py` for the native test, `relay.mjs` for the coding agent's) to a proxy set up as the app's is (loopback, plus the bridge gateway on the Linux runner), while the gateway itself isn't reachable from an agent directly | Layer: api (Docker) | Gap: two Nandi instances sharing the one relay (step 4) is manual.
- **Needs relay:** no
- **Status:** Shipped

### SB-15 — Coding agents through the egress proxy, with their model hosts for the run only

- **What:** Coding-agent containers (Claude Code, Codex, Gemini CLI) get the proxy env with their own proxy token (SB-11), so their `network.allow` is enforced like a native agent's. The hosts their CLI needs for its own model calls are allowed on top of `network.allow` only while the run lasts: Claude Code `api.anthropic.com` (or `*.amazonaws.com` with AWS credentials, or the Azure endpoint's host); Codex `api.openai.com`, `chatgpt.com`, `auth.openai.com`; Gemini CLI `generativelanguage.googleapis.com`, `cloudcode-pa.googleapis.com`, `oauth2.googleapis.com`; an external gateway's endpoint host; none through the LiteLLM gateway (it's on `nandi-agents`) or for a `custom:` CLI. When the run ends, however it ends, the hosts are taken back.
- **Where:** `services/orchestrator/runtime_router.rs` (`model_hosts`, `HarnessPlan.egress_hosts`, `execute_harness_run`, `SessionHostsGuard`), `services/egress/mod.rs` (`allow_session_hosts`, `clear_session_hosts`, `with_session_hosts`).
- **Preconditions:** Docker, `nandi-harness` built at `org.nandi.sandbox=5`, a Claude Code agent with an Anthropic key (and optionally Codex / Gemini CLI agents).
- **Steps:** Follow `docs/developer/sandbox-test-checklist.md` 4.6–4.9: 1. Ask the Claude Code agent anything. 2. Ask it to run `git ls-remote https://gitlab.com/gitlab-org/gitlab.git`. 3. After the run, check that `api.anthropic.com` is no longer allowed for that agent (egress log, or a native run under the same agent id). 4. Optional, without Docker runs: `python -I scripts/proxy-probe.py probe.log 120` and run each CLI with `HTTPS_PROXY=http://nandi:tok@nandi-egress:11435`, as in the checklist.
- **Expected:** 1. It answers; `egress.log` shows `api.anthropic.com` allowed for that agent. 2. Refused unless the agent's `network.allow` lists `gitlab.com`. 3. Not allowed after the run. 4. `probe.log` shows `CONNECT api.anthropic.com:443`, `api.openai.com`, `generativelanguage.googleapis.com`, each with `auth=yes`.
- **Edge / negative cases:** Gemini CLI also calls `play.googleapis.com` (telemetry), which the proxy refuses (checklist "Known gaps"); session hosts are keyed by agent id, so any token for that agent gets them while the run lasts; Bedrock via AWS keys opens all of `*.amazonaws.com` for the run.
- **Automation:** Covered by: `runtime_router.rs` (`coding_agents_leave_only_through_the_proxy_to_their_model`, gateway test asserting no `egress_hosts`), `egress/mod.rs` (`a_session_s_hosts_are_allowed_only_while_it_runs`); `sandbox_suite.rs` (`pnpm run test:sandbox`, the CI job `sandbox` on a Linux runner with Docker and the published images) `the_coding_agent_container_is_locked_down`: in the container a run plans (`runtime_router::plan`, `claude_code::env`), `git ls-remote https://example.org/…` goes through the relay and is refused by the proxy (`403`), and an allowed host is reached | Layer: api (Docker) + manual (a real CLI's model call) | Gap: the model hosts allowed for a real CLI's own calls (steps 1, 3, 4) need a sign-in and stay manual.
- **Needs relay:** no
- **Status:** Shipped

## Gaps and planned (not testable yet)

- No raw keys in coding-agent containers: they still receive `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `AWS_*` readable by agent code. Moved from Wave 1 to Wave 3, "model access through the gateway": every provider through the LiteLLM gateway with a per-run key; open question whether a Claude subscription sign-in works through a gateway (`docs/tasks-todo/task-x-enforced-policies.md`).
- Harness Test connection: falls back to the host when the image is missing, and its container isn't on `nandi-agents` (RT-10; checklist "Known gaps").
- Gemini CLI telemetry host `play.googleapis.com` is refused by the proxy (SB-15).
- Policy work still open (workflow steps, the gateway): see [policies.md](./policies.md) "Gaps and planned".
- Budgets per policy (tokens or money) (`task-x-enforced-policies.md`, "Later").
- Model picker per CLI runtime saved with `setHarnessDefault(id, model)`; UI always sends `null` (`docs/tasks-todo/task-x-models-and-runtimes.md` Wave A, unchecked item). Acceptance items (pick `gpt:` and see "Using GPT-5", Claude Code's model on its row, Build turns the card Ready) are unchecked.
- Prebuilt images pulled from GHCR (Build stays as fallback) (`task-x-models-and-runtimes.md` Wave C). No cancel for a running build.
- Custom ACP harness registration (`custom:<id>` needs a `harness_preferences` row; no command or UI writes one) (`services/harness/custom.rs`).
- Container harness turn completing on a Claude subscription token not verified; a host token with under 15 minutes left is refused.
- ChatGPT and Google OAuth logins not usable in the container (`docs/developer/harness-container.md`).
- `nandi:` model prefix (remote inference servers) and a Nandi-managed local model container are reservations only; `parse_model` rejects `nandi:` (`docs/developer/reserved-conventions.md`).
- Free trial: key is the same in every copy and recoverable from the binary; a Nandi-run proxy holding the key is future work; CI secret and DeepSeek spending cap still to set (`docs/developer/sign-in.md`, memory `free-trial-tokens.md`).
- Cost logging only for DeepSeek (`cost_usd` is `None` for Anthropic, OpenAI, Ollama, Claude Code) (`src-tauri/src/llm/*.rs`).
- `verify` does not consider Claude Code sign-in, the trial, or CLI runtimes/harness image (`src-tauri/src/verify.rs`).
- `scripts/e2e-smoke.ps1` only checks for a DeepSeek key (paths, schema and identifier were fixed on 2026-10-22).
- Model error messages from the core still say "Settings › Models" rather than "Settings › Models & runtimes" (`llm_trial` `TRIAL_USED_UP`, `llm_keys.rs` `NO_KEY`, `requests/blocker.rs`, `studio/validate.rs`).
- `list_containers` has no UI.
