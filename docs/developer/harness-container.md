# Harness Container

Agents with a harness `runtime:` run their CLI in `ghcr.io/vaultdbai/nandi-harness:sandbox-6`
(`runtimes/nandi-harness/`). It is Node plus the pinned harness CLIs, and
nothing of Nandi's own: no tool server. The CLI's model writes files and
runs commands with the CLI's built-in tools, and they execute in this
container against the agent's workdir.

Native agents keep using `ghcr.io/vaultdbai/nandi-runtime:sandbox-6` (the MCP tool server). The
two images are independent.

## Get the images (pull only)

Users never build an image. Both images are public on GitHub's container registry, published by `.github/workflows/images.yml`, and the app pulls what an agent needs the first time it needs it (`services/images/`).

| Tag           | When it's pushed                               | Who uses it                      |
| ------------- | ---------------------------------------------- | -------------------------------- |
| `sandbox-<N>` | Every publish; `<N>` is the Dockerfile's label | The app (pinned, never `latest`) |
| `main`        | Pushes to main that touch `runtimes/`          | Trying an unreleased image       |
| `<X.Y.Z>`     | A `v*` tag (the app version)                   | Pinning a release                |
| `latest`      | A `v*` tag                                     | People, not the app              |

- **One image service:** `images::ensure(SandboxImage)` is the only way the app gets one of its images: here and labelled for this sandbox, else pull the pinned tag (size and progress on `images://pull`), then check `org.nandi.sandbox` again. `runtime::start_container`, `runtime_router` (harness runs), the harness Test button and the connectors' containers all call it. A new image (say `nandi-connectors`) is a new `SandboxImage` variant (its tag and build context) and nothing else.
- **Size first:** `images::registry::download_size` reads the image's compressed size for this platform from the registry (an anonymous token from its `WWW-Authenticate` challenge) before the pull, so the UI says "Download now (450 MB)". Offline it's `None`, and the progress counts the layers seen.
- **Progress:** `images://pull` carries `{ image, build, line, progress: { doneBytes, totalBytes }, finished, error }`, progress at most every 250 ms. Settings › Models & runtimes shows it, and `SandboxProgressToasts` shows one toast per job, whichever view is open.
- **Failure:** a failed pull says what went wrong and to try again (Try again in the panel and the toast). It never falls back to a build.
- **Names in one place:** `runtime::DEFAULT_RUNTIME_IMAGE` and `runtime::HARNESS_IMAGE` (`ghcr.io/vaultdbai/<name>:sandbox-<SANDBOX_VERSION>`). Raise `SANDBOX_VERSION` and the Dockerfiles' `LABEL org.nandi.sandbox` together; a test checks they match. Sandbox 6: the harness image lost Claude Code and gained the tools volume below.
- **Build here (advanced, optional):** Settings › Models & runtimes › Advanced › "Build the sandboxes on this computer". The contexts are compiled into the app, and the build tags the same name, so the rest of the app doesn't care which. Nothing requires it.
- **Labels:** `org.opencontainers.image.source`, `title`, `description`, `licenses` (`Apache-2.0`; the CLIs in the harness image keep their own Apache-2.0 licences), `vendor=VaultDB.ai`, and `org.nandi.built-by` (the app version).
- **Platforms:** `linux/amd64` for now. Nandi Cloud (v2) needs arm64 too: add it when a native arm64 runner is available (QEMU makes the harness build slow).
- **Signing:** neither image is signed yet. Cosign verification and digest pinning are planned in the workspace task `task-x-v1-sandbox-images.md`.
- A business's own image builds `FROM ghcr.io/vaultdbai/nandi-runtime:sandbox-<N>` and inherits the label.

## What's in the harness image

| Package                     | Version | Licence    | ACP command             |
| --------------------------- | ------- | ---------- | ----------------------- |
| `@openai/codex`             | 0.163.0 | Apache-2.0 | (driven by `codex-acp`) |
| `@zed-industries/codex-acp` | 0.16.0  | Apache-2.0 | `codex-acp`             |
| `@google/gemini-cli`        | 0.64.0  | Apache-2.0 | `gemini --acp`          |

Plus git, ripgrep, Python 3 and uv (for connectors' `uvx` servers). Keep the versions in the Dockerfile and in `services/harness/*.rs` in step.

```bash
docker build -t ghcr.io/vaultdbai/nandi-harness:sandbox-6 runtimes/nandi-harness
docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-6 codex --version
docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-6 gemini --version
# No Claude Code in the image:
docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-6 sh -c 'ls /usr/local/lib/node_modules'
```

## Claude Code: the tools volume

Claude Code is **not** in the image. Anthropic's licence (`SEE LICENSE IN README.md`, its Commercial Terms) doesn't clearly allow redistributing it. The same goes for the Claude Agent SDK (`@anthropic-ai/claude-agent-sdk`, same terms), which the Apache-2.0 ACP adapter `@agentclientprotocol/claude-agent-acp` depends on: the SDK ships the Claude Code binary. So the adapter stays out too. The image ships only their lockfile, `runtimes/nandi-harness/claude-code/package-lock.json`, and the user's computer fetches them from npm on first use (`services/images/tools.rs`):

| Step      | What happens                                                                                                                                                                                                                                               |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| First use | A one-off harness container (as `node`, read-only root, `--cap-drop=ALL`, Docker's default network, the host's proxy variables by name) runs `install-tools.sh claude-code`: `npm ci` from the lockfile into the volume `nandi-tools-claude-code-<12 hex>` |
| Integrity | `npm ci` checks every package against the lockfile's `sha512` hashes. The script then writes the lockfile's sha256 to `.nandi-ready`                                                                                                                       |
| Check     | Once per app session, a `--network=none` container reads `.nandi-ready` and compares it with the app's embedded lockfile                                                                                                                                   |
| Runs      | `-v nandi-tools-claude-code-<hex>:/opt/nandi-tools/claude-code:ro` (`Placement::Container.tools`). The image's `PATH` starts with its `node_modules/.bin`. The root filesystem stays read-only                                                             |
| Update    | A new lockfile is a new volume name (its hash). After an install, the bundle's other volumes (label `org.nandi.tools=claude-code`) are removed; one still in use stays                                                                                     |

- The adapter runs the SDK's bundled Claude Code (`@anthropic-ai/claude-agent-sdk-linux-<arch>`), never a separate `@anthropic-ai/claude-code` package. So the lockfile pins `claude-agent-acp@0.93.0` and, through it, SDK 0.3.300 (Claude Code 2.1.300). The script removes the musl build npm also installs (the image is glibc).
- `images::tools::ensure` is called by `runtime_router::sandbox_ready` for `runtime: claude-code` (the run waits; progress on `images://tools`), by the harness Test button, and by Settings' **Get it now** / **Try again** (`install_tools`).
- The user accepts Anthropic's terms when they sign in to Claude Code, as before.
- To bump: change `ACP_VERSION` in `services/harness/claude_code.rs` and `claude-code/package.json`, then regenerate the lockfile in a container (`npm install --package-lock-only` in `node:22.20.0-bookworm-slim` with the folder mounted). A test checks the three agree and that every entry has an integrity hash.

## How a session runs

```text
docker run -i --rm --init --name nandi-harness-<tag>-<agent>
  --cap-drop=ALL --security-opt=no-new-privileges --pids-limit=512
  --user=1000:1000 --read-only --tmpfs=/tmp:… --tmpfs=/home/node:…
  -w /agent/workdir -v <workdir>:/agent/workdir
  [-v nandi-tools-claude-code-<hex>:/opt/nandi-tools/claude-code:ro]
  --network=nandi-agents -e NAME ... ghcr.io/vaultdbai/nandi-harness:sandbox-6 <acp command>
```

- stdin/stdout carry ACP (JSON-RPC, one message per line); stderr is kept
  (last 20 lines, never logged) to explain failures.
- Credentials are passed as `-e NAME` with no value. Docker reads the value
  from the `docker` process's own environment, so secrets never appear in a
  command line, a file, or a log.
- `--rm`: the CLI's home directory (sessions, caches) dies with the
  container.
- The root filesystem is read-only. `/tmp` and the home directory are
  tmpfs, so only the workdir and shared folders outlive the container.
- Codex's own OS sandbox can't nandi in Docker, so `codex-config.toml` sets
  `sandbox_mode = "danger-full-access"`. The container is the sandbox. The
  file sits in `/opt/nandi`; `entrypoint.sh` copies it into the home tmpfs
  on each start.
- The image carries `LABEL org.nandi.sandbox`. An image below
  `runtime::SANDBOX_VERSION` is pulled again before use (Settings › Models
  & runtimes shows "From an older Nandi").
- Agents never run on the host: `placement: host` in older files is
  ignored (`runtime_router::plan`) and rewritten to `container` on save.

## Credentials in the container

The container has no `~/.aws`, `~/.claude`, `~/.codex`, or `~/.gemini`.
What a session gets:

- **Bedrock**: `CLAUDE_CODE_USE_BEDROCK=1`, `AWS_REGION`, and either
  temporary STS keys resolved on the host from the user's AWS config (SSO,
  static keys, assume-role, credential_process) or `AWS_BEARER_TOKEN_BEDROCK`.
- **Claude Code subscription** (before any key): `CLAUDE_CODE_OAUTH_TOKEN`,
  the one in the environment or Claude Code's settings (from
  `claude setup-token`, long-lived), else the access token of the host's
  sign-in (`~/.claude/.credentials.json`, or the macOS keychain item
  `Claude Code-credentials`) while it has 15+ minutes left. The refresh
  token never leaves the host.
- **Anthropic, OpenAI, Gemini keys, last**: `ANTHROPIC_API_KEY` from
  Nandi's keyring only when Claude Code has no sign-in of its own;
  `OPENAI_API_KEY`, `GEMINI_API_KEY` from the keyring or the environment.
  With nothing to sign in with, a container run is refused before it
  starts.
- **Not supported in the container**: ChatGPT and Google OAuth logins.

The order and the reasons (no mounts of `~/.claude` or `~/.aws`) are in
`sign-in.md`. `HarnessHealth.auth` names the sign-in, and
`containerAuthenticated` says whether it reaches the container. Settings ›
Runtimes shows both, and Ready means the container can run and sign in.

**`max_turns`.** ACP has no turn limit, and a harness run is one ACP prompt
however many steps the CLI takes. `execute_harness_run` counts tool calls
(each is another model request) against `runtime_config.max_turns`, else
skill.md `max_turns` (default 50), and sends `session/cancel` and fails
the run when it goes over.

## LiteLLM gateway (third-party Bedrock models)

Claude Code speaks Anthropic Messages. For Llama, Mistral, DeepSeek, Nova,
and others on Bedrock, Nandi runs the official LiteLLM image
(`ghcr.io/berriai/litellm:v1.102.0`, `services/harness/gateway.rs`):

- Container `nandi-litellm` on the `nandi-harness` network, published on
  `127.0.0.1:4000`
- Config rendered from `litellm-config-template.yaml`. Model aliases
  `nandi-bedrock` and `nandi-bedrock-small` map to `bedrock/<model>`, and the
  file holds no secrets.
- AWS credentials and a per-start master key go into the container's
  environment. Claude Code gets `ANTHROPIC_BASE_URL=http://nandi-litellm:4000`
  and the master key as `ANTHROPIC_AUTH_TOKEN`.

```bash
docker pull ghcr.io/berriai/litellm:v1.102.0
```
