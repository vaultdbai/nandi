# Harness Container

Agents with a harness `runtime:` run their CLI in `ghcr.io/vaultdbai/nandi-harness:sandbox-5`
(`runtimes/nandi-harness/`). It is Node plus the pinned harness CLIs, and
nothing of Nandi's own: no tool server and no Python. The CLI's model writes
files and runs commands with the CLI's built-in tools, and they execute in
this container against the agent's workdir.

Native agents keep using `ghcr.io/vaultdbai/nandi-runtime:sandbox-5` (the MCP tool server). The
two images are independent.

## Get the images

**For now only `nandi-runtime` is published.** The harness image bundles Claude Code, and whether its licence allows a public image is still being checked, so `nandi-harness` is built locally (Settings › Models & runtimes › Build here, or the `docker build` below). Its Download button reports that the image isn't available.

The images are published to GitHub's container registry by `.github/workflows/images.yml` (only `nandi-runtime` for now) and pulled by the app (`services/images.rs`):

| Tag           | When it's pushed                               | Who uses it                      |
| ------------- | ---------------------------------------------- | -------------------------------- |
| `sandbox-<N>` | Every publish; `<N>` is the Dockerfile's label | The app (pinned, never `latest`) |
| `main`        | Pushes to main that touch `runtimes/`          | Trying an unreleased image       |
| `<X.Y.Z>`     | A `v*` tag (the app version)                   | Pinning a release                |
| `latest`      | A `v*` tag                                     | People, not the app              |

- **Names in one place:** `runtime::DEFAULT_RUNTIME_IMAGE` and `runtime::HARNESS_IMAGE` (`ghcr.io/vaultdbai/<name>:sandbox-<SANDBOX_VERSION>`). Raise `SANDBOX_VERSION` and the Dockerfiles' `LABEL org.nandi.sandbox` together; a test checks they match.
- **Pull by default:** Settings › Models & runtimes › **Download**, or by itself when a run or connector needs a missing image (`images::start_pull`; a native run waits with `images::pull`).
- **Build here** stays: the contexts are compiled into the app, and the build tags the same name, so the rest of the app doesn't care which. Offline or without the registry, a failed download says so and points to it.
- **Labels:** `org.opencontainers.image.source`, `title`, `description`, `licenses` (`Apache-2.0` for `nandi-runtime`; `Apache-2.0 AND LicenseRef-Bundled-CLIs` for `nandi-harness`, whose CLIs keep their own licences), `vendor=VaultDB.ai`, and `org.nandi.built-by` (the app version).
- **Platforms:** `linux/amd64` for now. Nandi Cloud (v2) needs arm64 too: add it when a native arm64 runner is available (QEMU makes the harness build slow).
- A business's own image builds `FROM ghcr.io/vaultdbai/nandi-runtime:sandbox-<N>` and inherits the label.

## Build

```bash
docker build -t ghcr.io/vaultdbai/nandi-harness:sandbox-5 runtimes/nandi-harness
docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-5 claude --version
docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-5 codex --version
docker run --rm ghcr.io/vaultdbai/nandi-harness:sandbox-5 gemini --version
```

| Package                                 | Version | ACP command                    |
| --------------------------------------- | ------- | ------------------------------ |
| `@anthropic-ai/claude-code`             | 2.1.302 | (driven by `claude-agent-acp`) |
| `@agentclientprotocol/claude-agent-acp` | 0.93.0  | `claude-agent-acp`             |
| `@openai/codex`                         | 0.163.0 | (driven by `codex-acp`)        |
| `@zed-industries/codex-acp`             | 0.16.0  | `codex-acp`                    |
| `@google/gemini-cli`                    | 0.64.0  | `gemini --acp`                 |

Keep the versions in the Dockerfile and in `services/harness/*.rs` in step.

## How a session runs

```text
docker run -i --rm --init --name nandi-harness-<tag>-<agent>
  --cap-drop=ALL --security-opt=no-new-privileges --pids-limit=512
  --user=1000:1000 --read-only --tmpfs=/tmp:… --tmpfs=/home/node:…
  -w /agent/workdir -v <workdir>:/agent/workdir
  --network=nandi-agents -e NAME ... ghcr.io/vaultdbai/nandi-harness:sandbox-5 <acp command>
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
- The image carries `LABEL org.nandi.sandbox`. The app refuses an image
  below `runtime::SANDBOX_VERSION` and asks for a rebuild (Settings ›
  Models & runtimes shows "Built for an older Nandi").
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
