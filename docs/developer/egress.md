# Egress Proxy

Agent containers reach the outside world only through the host's egress proxy (`src-tauri/src/services/egress/`). It listens on `127.0.0.1:11435`.

## The agents' network

Every agent container (Nandi's tool sandbox and the coding agents' one) joins only `nandi-agents`, a Docker network created `--internal`: no route to the internet or the host. Its one way out is the `nandi-egress` relay (`egress/relay.rs`), on that network and on Docker's default bridge, which forwards `nandi-egress:11435` to `host.docker.internal:<proxy port>`. The relay decides nothing; the proxy authenticates every request by its token. A process that ignores `HTTPS_PROXY` gets nowhere.

```text
agent ──nandi-agents──▶ nandi-egress:11435 ──bridge──▶ host proxy (127.0.0.1:11435)
```

- The relay runs from the image starting an agent (`relay.py` in `nandi-runtime`, `relay.mjs` in `nandi-harness`), read-only, no capabilities, no env. It's recreated when the proxy took another port (label `org.nandi.egress.target`).
- A pre-existing `nandi-agents` that isn't internal is refused, not used.
- The LiteLLM gateway joins `nandi-agents` too, so coding agents reach `nandi-litellm:4000` directly.
- Stdio MCP connectors run in their own container on `nandi-agents` too (`connectors/mcp.rs`, `Placement::Container`). Their proxy identity is `connector:<id>` (`ProxyTokens`, issued when the container starts). They may reach the package registries (`registry.npmjs.org`, `pypi.org`, `files.pythonhosted.org`) and the hosts in their `config.network` list in `connectors.yaml`, nothing else. A connector runs on the host only for agents whose policy lists it in `connectors.run_on_host`.
- Coding agents' own model calls go through the `CONNECT` tunnel. The hosts their provider needs (`api.anthropic.com`, `api.openai.com`, …; `runtime_router::model_hosts`) are allowed on top of `network.allow` while their run lasts (`egress::allow_session_hosts`).

| Route                          | Module          | What it does                                             |
| ------------------------------ | --------------- | -------------------------------------------------------- |
| `POST /llm`                    | `llm_proxy.rs`  | LLM completions with host-held provider keys             |
| `POST /aws/{service}/{action}` | `aws_proxy.rs`  | AWS calls as the agent's identity (MVP: S3 Get/Put/List) |
| `POST /fetch`                  | `http_proxy.rs` | HTTP requests to hosts in `network.allow`                |
| `CONNECT host:443`             | `tunnel.rs`     | HTTPS tunnels for tools that honor `HTTPS_PROXY`         |

## Rules

- **Identity comes from the token.** Every route needs the per-container proxy token (`Authorization: Bearer`, or the Basic password for `CONNECT`). A body `agent_id` that names another agent is refused with 403.
- **Capabilities are re-read per request** from the agent's skill.md, so edits apply immediately. Anything not listed is denied.
- **Secrets never leave the host or reach the logs.** The long-term AWS key stays in the keychain (`nandi.aws.access_key`, `nandi.aws.secret_key`, `nandi.aws.region`). `Debug` output redacts it, and so does the status UI (`AKIA…WXYZ`). Audit entries drop URL query strings and userinfo.
- **Redirects are returned, not followed**, so a redirect can't lead outside the allowlist.
- `aws`, `fetch`, and `tunnel` requests (allowed or denied) are appended to `~/.nandi/local/logs/egress.log`. LLM calls keep going to `llm.log`.

## skill.md capabilities

```yaml
aws:
  mode: proxy # default. `sts` also injects scoped credentials at container start
  services:
    - service: s3
      actions: [GetObject, PutObject, ListObjects]
      resources: ['arn:aws:s3:::reports', 'arn:aws:s3:::reports/*']
network:
  allow:
    - host: api.github.com
      methods: [GET] # omit for any method
    - host: '*.example.com' # subdomains only
    - host: 'localhost:8080' # non-default port
```

- `host: '*'` allows any site (still through the proxy, still audited).
- Users set the hosts on the agent page (Settings › Internet access: none, a list, or any site; `NetworkAccess.tsx`) or with **Allow** on a blocked request in Settings › Internet access (`useAllowSite`). Both save through `repo_save_agent`, which edits hosts only: each kept host keeps its `methods`.
- Omitting `methods` allows any method; omitting `resources` allows any resource of the listed actions.
- `ListObjects` is authorized by IAM's `s3:ListBucket` on the **bucket** ARN, so list both `arn:aws:s3:::bucket` and `arn:aws:s3:::bucket/*`.
- **Tunnels can't see methods** inside TLS, so `CONNECT` is allowed only for hosts whose rule has no `methods` limit. Method-limited hosts must use `/fetch`. Tunnels go to port 443 unless the rule names a port. `sts` agents can also tunnel to `*.amazonaws.com`.
- Parsed capabilities are stored in `agent_state.capabilities` (on start and snapshot) and returned as `AgentSnapshot.capabilities` for the Permissions panel.

## AWS sessions

`CredentialsManager::session(agent_id, profile, policy, duration, usage)` picks the agent's identity, in order:

1. `aws.profile` in its skill.md (chosen on the agent's Settings tab).
2. The keychain key (Settings → Cloud credentials → Access key).
3. The sign-in profile there (`AWS_PROFILE`, else the SDK's default chain).

Profiles resolve on the host through `aws_auth::agent_keys` (the same SDK chain Bedrock uses), wired in at start-up with `set_sign_in`.

- **Long-term keys** (the keychain key, or a profile's static keys) go through STS **GetFederationToken**, so no role ARN is needed. The session gets the intersection of the key's permissions and a session policy generated from the agent's grants (`Capabilities::session_policy`).
- **Temporary keys** (SSO, assume-role, `credential_process`) can't call GetFederationToken, and narrowing them would need a role to assume. They're used as they are for `SessionUse::Proxy` only: the proxy's grant check is then the only one. `SessionUse::Container` refuses them, so an `sts` agent never holds keys wider than its grants.

Sessions are cached per `(agent_id, sha256(policy), profile)` until 60s before they expire, with whether they're narrowed: a proxy-only session is never handed to a container. Changing the key or region clears the cache. The region is the key's (Settings) or the profile's.

- Proxied calls use 1-hour sessions. With long-term keys, AWS enforces the grants a second time, after the proxy's own check.
- `sts`-mode containers get 12-hour sessions as `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` / `AWS_REGION`. A container that outlives its session needs a restart. Starting an `sts` agent fails without a key or long-term profile keys.

## Container environment

`runtime::start_container` takes its proxy env from `EgressState::container_env`; harness containers get the same (`egress::proxy_env`, through `runtime_router::plan`):

| Variable                                    | Value                                            |
| ------------------------------------------- | ------------------------------------------------ |
| `NANDI_LLM_PROXY`, `NANDI_LLM_PROXY_URL`    | `http://nandi-egress:11435/llm`                  |
| `NANDI_LLM_PROXY_TOKEN`                     | Per-container proxy token                        |
| `NANDI_AWS_PROXY`                           | `http://nandi-egress:11435/aws`                  |
| `NANDI_FETCH_PROXY`                         | `http://nandi-egress:11435/fetch`                |
| `HTTPS_PROXY`, `HTTP_PROXY` (and lowercase) | `http://nandi:<token>@nandi-egress:11435`        |
| `NO_PROXY`, `no_proxy`                      | `nandi-egress,nandi-litellm,localhost,127.0.0.1` |

## Testing

Handlers take injectable dependencies (capability loader, AWS executor, STS issuer), so `cargo test` covers authentication, allow/deny decisions, STS caching and expiry, redaction, and a real `CONNECT` tunnel through the server, all without AWS or Docker.
