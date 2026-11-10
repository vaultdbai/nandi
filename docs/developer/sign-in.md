# Sign-in: Company Identity First, Keys Last

Nandi is for teams. Model calls and coding agents sign in with what the
user already has from their company: a Claude account login (Team or
Enterprise) or AWS (SSO, SAML, assume-role). A pasted API key is the last
resort.

Two rules shape this:

- **Agents' code runs only in the sandbox container.** A
  `runtime: claude-code` agent runs Claude Code inside `nandi-harness`, not
  on this computer. Only connectors and pilots reach outside, through
  Nandi.
- **No host config folder is mounted into the container.** `~/.claude`
  and `~/.aws` stay on the host. The container gets only what one session
  needs to sign in, through its environment.

## Coding agents in the container

`runtime_router::credentials` and `claude_code::ClaudeCode::env`, in order:

1. **The team's or agent's Bedrock choice** (`.nandi/llm.yaml`
   `provider: bedrock`, or `runtime_config.provider: bedrock`). The host
   resolves the AWS profile with the AWS SDK's own chain (SSO cache, SAML
   or `credential_process` helpers, assume-role, keys) into temporary STS
   keys (`aws_auth::bedrock_credentials`). Only those keys and
   `CLAUDE_CODE_USE_BEDROCK=1` enter the container. An expired SSO session
   fails the run with `aws sso login --profile <p>`.
2. **Claude Code's own setup**: the provider switches in its settings
   (Bedrock, Vertex, Foundry, including IT-managed ones), or the host
   login's access token as `CLAUDE_CODE_OAUTH_TOKEN`. The token comes from
   `~/.claude/.credentials.json`, or on macOS from the `Claude
Code-credentials` keychain item. It must have 15+ minutes left. The
   refresh token never leaves the host.
3. **Nandi's Anthropic key**, only when 2 found nothing.

Why not mount `~/.claude` or `~/.aws` read-only: Claude Code writes to its
folder and refreshes its login there, so a read-only mount breaks it on
the first refresh. A writable one lets the container rotate the host's
refresh token and sign the host out. It would also expose every
transcript and project in it. SAML and `credential_process` helpers
aren't in the image, and the SSO cache can't refresh read-only.

## Nandi's own model calls

Studio, chat, compaction, and native agents' turns go through `llm`. A
`claude:` model goes through Claude Code when it can sign in
(`harness::claude_sdk::usable`), else the Anthropic key. With nothing else
set up, every call uses Claude Code (`llm_keys::usable_model`).

- `claude_sdk::complete` runs `runtimes/claude-sdk/bridge.mjs` under the
  user's Node. It uses the Claude Agent SDK
  (`@anthropic-ai/claude-agent-sdk`, pinned) with the user's own `claude`
  executable. The SDK is installed on first use into
  `~/.nandi/local/runtimes/claude-sdk` with `npm install --omit=optional`,
  without its bundled CLI.
- A call is a plain completion: `tools: []`, `mcpServers: {}` with
  `strictMcpConfig`, `settingSources: []` (no hooks, no project files),
  one turn, in an empty folder. Nothing an agent wrote runs on the host.
- Sign-in follows the same order: the team's Bedrock choice passes
  `CLAUDE_CODE_USE_BEDROCK=1`, `AWS_PROFILE`, and `AWS_REGION`, and Claude
  Code resolves the profile from `~/.aws` itself. Otherwise Claude Code
  signs in its own way.
- Nandi's tools are described in the system prompt, and the model calls
  them with `<tool_call>{"name", "arguments"}</tool_call>` blocks
  (`llm::claude_code`). Nandi runs them as it does for every provider.
- Model names map to Claude Code aliases (`claude-opus-5` → `opus`), which
  resolve to what the user's plan or Bedrock offers.

## The free tokens (trial)

A new user can try Nandi before setting up a model: Nandi's trial
gateway (`https://trial.vaultdb.ai`, private repo
`nandi-cloud/trial-gateway`) holds the DeepSeek key and counts tokens
(`services::llm_trial`). **The app holds no key**: nothing is baked into
builds, no CI secret, no fallback.

- **Allowance:** 100,000 tokens (input + output) per person on
  `deepseek:deepseek-flash` only. A device nobody signed in on counts as
  one person; signing in links devices so they share one allowance. The
  gateway is the only counter: the app keeps none, and Settings shows
  what `GET /v1/trial` says (`get_free_trial_account`).
- **Last resort:** `llm_keys::usable_model` tries the user's own keys and
  sign-ins first. The trial runs only when nothing else works, only on
  the host, never in a container.
- **Off once the user has their own model:** the first time a key, a
  Claude Code sign-in, or Ollama as the default works, the trial turns
  off for good (`nandi.trial.deepseek.mode = off`). Saving a key turns it
  off even after the user turned it back on.
- **Back on by choice:** Settings › Models › Free tokens › **Use free
  tokens** (`set_free_trial`) works unless the gateway said the tokens are
  used up. It makes the trial model the default, and `deepseek:` calls use
  the trial before the user's DeepSeek key.
- **Gateway URL:** `llm_trial::gateway::DEFAULT_GATEWAY_URL`;
  `NANDI_TRIAL_GATEWAY_URL` overrides it (staging, a local gateway). The
  trial is set up at start (`llm_trial::init`) with the instance ID.
  Tests never reach it: they see only a trial they set on their thread
  (`llm_trial::use_test_trial`, usually a fake axum gateway).

### Signing

Each request carries `X-Nandi-Instance` (the instance ID),
`X-Nandi-Key` (the device's Ed25519 public key, `relay::device_keys`, made
on first use and pinned by the gateway like the relay pins it),
`X-Nandi-Time` (epoch ms) and `X-Nandi-Signature`
(`DeviceKeys::sign`) over:

```text
nandi-trial-v1\n<METHOD>\n<path>\n<instance>\n<time>\n<hex sha256(body)>
```

The body is serialized once and those bytes are hashed, signed and sent.
Signed times always go up (the gateway refuses replays).
`gateway_tests::the_signing_string_matches_the_gateway` pins the
gateway's own golden string.

### Calls and errors

- `POST /v1/chat/completions`: `deepseek::request_body` on the trial
  model, no bearer key; the answer is DeepSeek's stream, read by
  `deepseek::read_stream`.
- The trial path doesn't use the DeepSeek adapter's retries. A
  `Retry-After` of 10 s or less on `trial_busy`, `rate_limited` or
  `upstream_busy` is waited out once; longer ones are reported.
  `stale_time` adjusts the clock by `serverTime` and signs again once.
- `GatewayError::user_message` turns each code into a sentence with what
  to do (used up, not enough left, paused until about when, rate limited,
  too many devices, clock, key mismatch, model busy or refused).
- What the gateway last said is kept in memory (`Trial::note`): used up,
  paused until `Retry-After`, unreachable for a minute. While any holds
  the trial isn't offered, `effectiveModel` is `null`, and the Ask box
  answers from the help guide (`machine-readiness.md`) without waiting on
  a timeout.

### Linking devices (sign-in)

`POST /v1/signin/<provider>/start` returns a user code and a
verification URL; the app opens it and polls `/poll` every `interval`
seconds (`pending`, `slow_down`, `linked`, or an error that stops it).
The provider's device code stays on the gateway. Settings shows a button
for each provider `GET /v1/trial` lists in `signIn` (GitHub today);
Google and Microsoft show as "coming soon" until the gateway offers them
(it answers `provider_not_supported`).

## What the UI shows

`HarnessHealth.auth` (`detection::auth_of`) names the sign-in without
secrets: `subscription` (with plan and organization), `bedrock` (with
profile and region), `vertex`, `azure`, `gateway`, or `apiKey`.

- **Settings › Runtimes**: per card, whether it runs in the sandbox (image
  built), what it signs in with, and whether that reaches the sandbox.
  Ready means both. An expired SSO session shows the `aws sso login`
  command, and Re-check sign-in detects again. A key is only offered as
  "Use an API key instead".
- **Settings › Cloud credentials › Sign-in**: the profile picker, and
  the sign-in each profile has: **Log in** (`aws sso login`, or `aws login`
  for a `login_session` profile where the CLI has it), **Replace keys** for
  an access-key profile. **Add SSO profile** and **Add access keys** set
  up new ones (below).
- **Settings › Models**: `DefaultModelStatus.claudeSignIn` says which
  sign-in Claude calls use. With one, the Anthropic key form waits behind
  "Use an API key instead".

## Setting up AWS profiles

`aws_auth::setup`. Nandi still writes no AWS file itself: the AWS CLI
does, on whatever files it uses (`AWS_CONFIG_FILE`, `~/.aws`).

- **Access keys**: `aws configure --profile <name>` with the four answers
  on stdin (key ID, secret, region, output). The secret is never on a
  command line, in a log, or in `Debug` (`KeyProfileInput`). A blank
  region keeps the profile's.
- **SSO** (IAM Identity Center), three steps:
  1. `start_aws_sso_setup`: SSO OIDC `RegisterClient` (public client
     "Nandi") and `StartDeviceAuthorization` for the start URL. The UI
     opens the verification page with the code filled in.
  2. `poll_aws_sso_setup`: `CreateToken` at the interval AWS asked for
     (+5s after a SlowDown). Once approved, the SSO portal API lists the
     accounts and roles (first 100 accounts). The token lives only in
     memory and is dropped then.
  3. `finish_aws_sso_setup`: only an account and role from that list.
     `aws configure set` writes `sso_start_url`, `sso_region`,
     `sso_account_id`, `sso_role_name`, `region`, `output`. Then the UI
     runs `aws sso login` for the profile (a second, one-click approval),
     because the SDK reads the sign-in from the CLI's cache.
- `aws configure set` can't write `sso-session` sections (it writes the
  value into `[default]`), so new profiles use the profile's own `sso_*`
  keys. The CLI and the SDK both read them; they don't refresh the token
  on their own, so the user logs in again when it expires.
- **`aws login`** (console sign-in): profiles with `login_session` are
  `consoleLogin`. `AwsCliInfo.supportsLogin` asks the CLI (`aws login
help`), not its version. The SDK resolves them with aws-config's
  `credentials-login` feature.
