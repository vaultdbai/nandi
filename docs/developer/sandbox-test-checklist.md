# Sandbox and Policy Test Checklist

Manual and Docker checks for the enforced-policies work (`docs/tasks-todo/task-x-enforced-policies.md`): the hardened containers and harness approvals (#8), the agents' network (#9), the policy format (wave 2), policies enforced on every run and shown on the agent page (waves 3 and 4), and connectors in containers (wave 3b). Unit tests cover the logic; these cover what only a real Docker, CLI and UI show.

Each check says what to do, what you should see, and where the code is. Tick it in the PR or the test log when it passes.

## Before you start

1. Docker Desktop running (`docker version` answers). If it returns "Internal Server Error", restart Docker Desktop.
2. Rebuild both sandbox images from **Settings › Models & runtimes › Rebuild** (or `docker build -t ghcr.io/vaultdbai/nandi-runtime:sandbox-5 runtimes/nandi-runtime` and `docker build -t ghcr.io/vaultdbai/nandi-harness:sandbox-5 runtimes/nandi-harness`). Both must carry `org.nandi.sandbox=5`:

   ```powershell
   docker image inspect ghcr.io/vaultdbai/nandi-runtime:sandbox-5 ghcr.io/vaultdbai/nandi-harness:sandbox-5 --format "{{json .Config.Labels}}"
   ```

3. For the coding-agent checks: an Anthropic, OpenAI or Gemini key in Settings, and an agent with `runtime: claude-code` (or `codex`, `gemini-cli`).

## 1. Images and hardening (#8)

| #   | Do                                                                                                                                       | Expect                                                                                                                                                                                 | Code                                           |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| 1.1 | Tag an old image over the new one (`docker tag <old id> ghcr.io/vaultdbai/nandi-runtime:sandbox-5`), open Settings › Models & runtimes   | "Built for an older Nandi (ghcr.io/vaultdbai/nandi-runtime:sandbox-5). Rebuild it so agents can use it." with **Rebuild**; running a native agent with tools fails with a rebuild hint | `runtime::image_is_current`, `ImageBuildPanel` |
| 1.2 | Run a native agent with `shell`; ask it to run `id; touch /usr/x; touch /tmp/x && echo tmp-ok`                                           | `uid=1000`; `/usr/x` fails (read-only file system); `tmp-ok`                                                                                                                           | `runtime::container_config`                    |
| 1.3 | `docker inspect nandi-agent-<tag>-<id> --format "{{.HostConfig.ReadonlyRootfs}} {{.Config.User}} {{.HostConfig.CapDrop}}"` while it runs | `true 1000:1000 [ALL]`                                                                                                                                                                 | same                                           |
| 1.4 | Run a Claude Code agent; ask it to run `id; touch /usr/x`                                                                                | `uid=1000(node)`; `/usr/x` fails                                                                                                                                                       | `harness/adapter.rs` `docker_args`             |
| 1.5 | Run a Codex agent once                                                                                                                   | It starts (Codex's config was copied into the home tmpfs by `entrypoint.sh`)                                                                                                           | `runtimes/nandi-harness/entrypoint.sh`         |

## 2. No host placement (#8)

| #   | Do                                                                                           | Expect                                                                           | Code                   |
| --- | -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ---------------------- |
| 2.1 | Agent page › Settings for a Claude Code agent                                                | "Where it runs" is text, "In an isolated container…"; no Host / Container choice | `AgentSettingsTab.tsx` |
| 2.2 | With only a Claude Code sign-in on this computer (no Anthropic key), run a Claude Code agent | The run fails with "Claude Code needs an API key to run in the container…"       | `runtime_router::plan` |

## 3. Coding agents ask first (#8)

Use a Claude Code agent with no `requires_approval` in skill.md (the default asks for `shell` and `write_file`).

| #   | Do                                                                                                       | Expect                                                                                                      | Code                                             |
| --- | -------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| 3.1 | Ask it to create `notes.txt`                                                                             | An approval appears in the Inbox and chat (tool `write_file`, the CLI's title under Details); the run waits | `harness/approvals.rs`, `acp.rs`                 |
| 3.2 | Approve                                                                                                  | The file is written; the run goes on and finishes                                                           | `approvals::decide`                              |
| 3.3 | Ask again and Reject                                                                                     | The CLI is told no and says so; nothing written                                                             | same                                             |
| 3.4 | Ask it to run a command and leave the approval for an hour (or shorten `APPROVAL_TTL_MS` in a dev build) | The approval expires and the CLI is told no                                                                 | `RunApprover::ask`                               |
| 3.5 | Ask for a command, then quit Nandi while it waits; start Nandi and decide the approval                   | Deciding says the run stopped while it waited; the run shows Cancelled, not restarted                       | `approvals::close_stale`, `chat::approve_action` |
| 3.6 | Cancel a run while its approval is pending                                                               | The approval closes as expired; no stray approval left in the Inbox                                         | `turn_done`                                      |
| 3.7 | Set `requires_approval: []`, ask it to write a file                                                      | No approval; the run's Approvals tab says nothing needed approval and lists what was allowed                | `RunDetailTabs.tsx` `RuntimeDecisions`           |
| 3.8 | Studio form and agent Settings for a coding agent                                                        | No "Approvals aren't enforced" note anywhere                                                                | `AgentForm.tsx`, `AgentSettingsTab.tsx`          |

## 4. The agents' network (#9)

| #    | Do                                                                                                                                                                                                                                                 | Expect                                                                                                                               | Code                                  |
| ---- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------- |
| 4.1  | `cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_e2e -- --ignored --nocapture`                                                                                                                                                       | Passes: tools over `docker exec`, workdir confinement, no direct internet, read-only root, no keys in env                            | `mcp_client.rs` `runtime_e2e`         |
| 4.2  | Run any agent, then `docker network inspect nandi-agents --format "{{.Internal}}"` and `docker ps --filter name=nandi-egress`                                                                                                                      | `true`; `nandi-egress` running                                                                                                       | `egress/relay.rs`                     |
| 4.3  | Native agent with `network: { allow: [{ host: api.github.com }] }`; ask it to run `python -c "import urllib.request as u; print(u.urlopen('https://api.github.com').status)"`, then the same for `https://example.com` (the images have no `curl`) | The first answers; the second is refused by the proxy; both show in Settings › Internet access / Logs (egress log), allowed / denied | `egress/tunnel.rs`                    |
| 4.4  | Same agent: the same Python line under `env -u HTTPS_PROXY -u https_proxy -u HTTP_PROXY -u http_proxy`                                                                                                                                             | Fails (no route): skipping the proxy goes nowhere                                                                                    | `relay::ensure_ready`                 |
| 4.5  | `docker port nandi-agent-<tag>-<id>`                                                                                                                                                                                                               | Nothing published                                                                                                                    | `runtime::container_config`           |
| 4.6  | Claude Code agent with an Anthropic key: ask it anything                                                                                                                                                                                           | It answers (its `api.anthropic.com` calls go through the proxy); the egress log shows `api.anthropic.com` allowed for that agent     | `runtime_router::model_hosts`         |
| 4.7  | Same agent: ask it to run `git ls-remote https://gitlab.com/gitlab-org/gitlab.git` (git honors `HTTPS_PROXY`)                                                                                                                                      | Refused by the proxy unless the agent's `network.allow` lists it                                                                     | `egress::allow_session_hosts`         |
| 4.8  | After that run ends, a direct check that `api.anthropic.com` isn't left open: run a native agent of the same id, or read the egress log                                                                                                            | Model hosts are only allowed while the run lasts                                                                                     | `SessionHostsGuard`                   |
| 4.9  | Quit Nandi, start the proxy stand-in (`python -I scripts/proxy-probe.py probe.log 120`), run each CLI as in `harness-container.md` with a dummy key and `HTTPS_PROXY=http://nandi:tok@nandi-egress:11435`                                          | `probe.log` shows `CONNECT api.anthropic.com:443`, `api.openai.com`, `generativelanguage.googleapis.com`, each with `auth=yes`       | `scripts/proxy-probe.py`              |
| 4.10 | Bedrock through the LiteLLM gateway (`runtime_config: { gateway: litellm }`)                                                                                                                                                                       | The agent answers; `docker inspect nandi-litellm` shows it on `nandi-harness` and `nandi-agents`                                     | `harness/gateway.rs`                  |
| 4.11 | Start a second Nandi with another `NANDI_HOME` (its proxy takes another port), run an agent                                                                                                                                                        | The relay is recreated for the new port and the agent still reaches out                                                              | relay label `org.nandi.egress.target` |
| 4.12 | Create a non-internal network named `nandi-agents` by hand, run an agent                                                                                                                                                                           | The run fails saying the network isn't internal; nothing starts                                                                      | `relay::ensure_network`               |

## 5. Policies (wave 2: format and evaluator)

Check the policy format through Studio and the unit tests; section 6 checks what runs do with it.

| #   | Do                                                                           | Expect                                                                                                | Code                  |
| --- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | --------------------- |
| 5.1 | `cargo test --manifest-path src-tauri/Cargo.toml --lib domain::policy`       | All pass                                                                                              | `domain/policy.rs`    |
| 5.2 | In Studio, save a policy with `tools: { deny: [shell] }` and no `applies_to` | Refused: its rules would apply to nobody                                                              | `studio/validate.rs`  |
| 5.3 | Save one with `applies_to: [robot:x]`, or `tools: { block: [shell] }`        | Refused with the bad target or key named                                                              | same                  |
| 5.4 | Save an existing prose policy (only `name:` and text)                        | Saves as before; it applies nothing                                                                   | `Policy::is_enforced` |
| 5.5 | Put a broken policy file in `.nandi/policies/` by hand                       | The agents it names don't run, with the policy's name and error; unreadable targets block every agent | `PolicySet::blocking` |

## 6. Policies enforced (wave 3)

Write each policy as `.nandi/policies/<id>.md` (or through Studio), then run the agent. Remove it afterwards.

| #    | Policy                                                                                                    | Do                                    | Expect                                                                                            | Code                                |
| ---- | --------------------------------------------------------------------------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------- | ----------------------------------- |
| 6.1  | `applies_to: [agent:<id>]`, `tools: { deny: [shell] }`                                                    | Native agent: ask it to run a command | `shell` isn't offered; a forced call says "a policy doesn't allow it"                             | `RepoStore::skill`, `orchestrator`  |
| 6.2  | Same, on a Claude Code agent                                                                              | Ask it to run a command               | The CLI's command request is rejected; edits still work (after approval)                          | `PermissionPolicy::Except`          |
| 6.3  | `tools: { ask: [read_file] }`                                                                             | Ask it to read a file                 | An approval appears for `read_file` (native) or for reads (coding agent)                          | `policies::narrow`                  |
| 6.4  | `network: { allow: [{ host: api.github.com }] }` on an agent that allows `*.github.com` and `example.com` | Fetch both hosts                      | `api.github.com` works; `example.com` and `github.com` are refused by the proxy                   | egress capability lookup            |
| 6.5  | `files: [{ folder: <id>, mode: read }]` on an agent with that folder as write                             | Write a file there                    | Refused (read-only mount and tool); a folder not listed isn't mounted at all                      | `runtime::folder_mounts`            |
| 6.6  | `models: { allow: ['gpt:*'] }` on a native agent using a Claude model                                     | Run it                                | The run fails: "The policy '…' doesn't allow the model …"                                         | `policies::narrow`                  |
| 6.7  | `connectors: { allow: [] }` on an agent with a connector                                                  | Run it                                | The connector's tools aren't offered                                                              | `orchestrator::declared_connectors` |
| 6.8  | `pilots: { allow: [] }` on an agent with the browser pilot                                                | Ask it to use the browser             | No pilot tools; a forced call says "a policy doesn't allow this pilot"                            | same                                |
| 6.9  | Two policies, one `tools: { allow: [shell] }`, one `tools: { deny: [shell] }`                             | Run it                                | `shell` is denied (deny wins)                                                                     | `domain::policy::effective`         |
| 6.10 | `models: { allow: ['claude:sonnet*'] }` on a Claude Code agent with `runtime_config.model: opus`          | Run it                                | The run fails before its container starts: "The policy '…' doesn't allow the model claude:opus …" | `runtime_router::plan`              |
| 6.11 | `applies_to: [tag:finance]`, `tools: { deny: [shell] }`; add `tags: [finance]` to one agent's skill.md    | Ask both agents to run a command      | The tagged agent can't run `shell`; an untagged agent still can                                   | `Agent::tags`, `policies::evaluate` |
| 6.12 | Any policy above, with the agent page › Settings › Permissions open                                       | Save the policy in Studio             | Without a reload: "Policies that apply: …" and one row per change, "by <policy>"                  | `AgentPolicies.tsx`, `agent-sync`   |

## 7. Connectors in containers (wave 3b)

Use an MCP connector in `connectors.yaml`, for example `{id: everything, kind: mcp, config: {command: npx, args: [-y, '@modelcontextprotocol/server-everything']}}`, declared by an agent.

| #   | Do                                                                                                       | Expect                                                                                                                  | Code                                |
| --- | -------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| 7.1 | `cargo test --manifest-path src-tauri/Cargo.toml --lib connector_container_e2e -- --ignored --nocapture` | Passes: the server runs as uid 1000 on `nandi-agents`, read-only, with `HTTPS_PROXY` set                                | `connectors/mcp.rs`                 |
| 7.2 | Run the agent; `docker ps --filter name=nandi-connector-`                                                | `nandi-connector-everything` is running; `docker inspect` shows `--read-only`, user `1000:1000`, network `nandi-agents` | `container_args`                    |
| 7.3 | Watch Settings › Internet access while it starts                                                         | `registry.npmjs.org` allowed for `connector:everything`; nothing else unless listed in `config.network`                 | `egress::connector_capabilities`    |
| 7.4 | Add `network: [api.github.com]` under the connector's `config`; have a tool reach it                     | Allowed; any other host refused by the proxy                                                                            | same                                |
| 7.5 | A policy with `connectors: { run_on_host: [everything] }` for this agent; run it                         | No container for it; the server runs as a host process for this agent only (another agent still gets the container)     | `ConnectorRegistry::start_placed`   |
| 7.6 | Stop the connector in Library › Browse › Tools                                                           | Both the container and any host instance stop                                                                           | `ConnectorRegistry::stop`           |
| 7.7 | A `uvx` connector (for example `uvx mcp-server-time`)                                                    | Starts in the container (Python and uv are in `nandi-harness`)                                                          | `runtimes/nandi-harness/Dockerfile` |

## Known gaps (not tested here)

- API keys still go into coding-agent containers (wave 3, model access through the gateway).
- The harness **Test** button falls back to the host when the image is missing, and its container isn't on `nandi-agents`.
- Gemini CLI also calls `play.googleapis.com` (telemetry), which the proxy refuses.
