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

Release builds ship Nandi's own DeepSeek key so a new user can try
Nandi before setting up a model (`services::llm_trial`).

- **Allowance:** 100,000 tokens (input + output) per computer, on
  `deepseek:deepseek-flash` only. The count lives in the keychain
  (`nandi.trial.deepseek.used`), so clearing `~/.nandi` doesn't reset it.
- **Last resort:** `llm_keys::usable_model` tries the user's own keys and
  sign-ins first. The trial runs only when nothing else works.
- **Off once the user has their own model:** the first time a key, a
  Claude Code sign-in, or Ollama as the default works, the trial turns
  off for good (`nandi.trial.deepseek.mode = off`). Saving a key turns it
  off even after the user turned it back on.
- **Back on by choice:** Settings › Models › Free tokens › **Use free
  tokens** (`set_free_trial`) works while tokens are left. It makes the
  trial model the default, and `deepseek:` calls use the trial before the
  user's DeepSeek key.
- **The key:** CI passes `NANDI_TRIAL_DEEPSEEK_KEY` (a repository
  secret) to the build. `build.rs` XORs it with a pad made for that build,
  so it isn't plain text in the binary. It is unmasked in memory only for
  a trial call, never written to disk, and never sent into a container.
  Once the tokens are used up the app never unmasks it again. A build
  without the variable has no trial, and tests never see one.
- **Limits of this design:** the key is the same in every copy of the
  app, and someone who takes the binary apart can recover it. Keep a
  spending cap on the DeepSeek account and rotate the key if it leaks. A
  per-user limit that can't be bypassed needs a Nandi-run proxy that
  holds the key; the trial is shaped so `deepseek::stream` could point at
  one later.

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
