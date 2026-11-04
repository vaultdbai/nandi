# Task: One auth layer for everything, then any MCP server with any standard auth

**From:** owner, 2026-10-30:

- "Can we add external MCP servers so Nandi can talk to them, with all standard auths supported?"
- "Put all common auth components in one place, so every feature uses it instead of each one implementing its own."

**Size:** L, about 3 to 4 weeks for one developer, in four phases. Each phase ships on its own.
**After the demo.** Nothing here blocks it.
**Replaces:** `task-x-mcp-auth.md`. Its two parts are phases 2 and 3 below.
**Feeds:** `task-x-v2-sign-in.md`. Company sign-in (Entra ID, GitHub org) should be a provider on this layer, not a new flow. `task-x-outlook-connector.md` needs phase 3 (device-code token cache).

## Where things stand (survey 2026-10-30)

Keychain access is already in one place: `services/secrets.rs`. Everything above it is built separately in each area.

| Area                | What it does today                                                                                                                                                                                                                                                                                                                                 | Files                                                                                                                                                                                                                   |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Connector OAuth     | PKCE with a one-shot localhost redirect, plus the device flow (RFC 8628). Tokens are stored as JSON with `refresh_token` and `expires_at`, but **nothing refreshes them**. The only provider is Gmail, still a stub                                                                                                                                | `connectors/oauth.rs`                                                                                                                                                                                                   |
| App GitHub sign-in  | The `oauth.rs` device flow, with its own flow registry. `nandi.github.token`                                                                                                                                                                                                                                                                       | `auth/github.rs`                                                                                                                                                                                                        |
| GitHub connector    | Three token sources: a pasted token (`connectors.github.token`, the only entry without the `nandi.` prefix), the app sign-in, then `gh auth token`. Resolved separately in `catalog.rs`, `marketplace/mod.rs` (`GITHUB_SIGN_IN_REF`) and `remote`                                                                                                  | `connectors/github.rs`, `catalog.rs:265-290`                                                                                                                                                                            |
| AWS                 | A **second device-flow implementation** (SSO OIDC; the token is kept in memory only). STS for Bedrock. A separate `CredentialsManager` for egress that tries the agent's profile, then keychain keys with GetFederationToken, then the sign-in profile. The connectors call the AWS CLI with `--profile`. The identity is worked out in two places | `harness/aws_auth.rs`, `harness/aws_auth/setup.rs`, `egress/credentials.rs`, `connectors/aws.rs`, `s3_backup.rs`                                                                                                        |
| LLM keys            | Set, test and status checks; a key can never be read back. Trial key masked at build time                                                                                                                                                                                                                                                          | `llm_keys.rs`, `llm_trial.rs`                                                                                                                                                                                           |
| Runtimes            | Claude login passed in as `CLAUDE_CODE_OAUTH_TOKEN`, read from the host's credentials file. Codex, Gemini and custom runtimes use environment API keys only                                                                                                                                                                                        | `harness/claude_code.rs`, `detection.rs`, `codex.rs`, `gemini_cli.rs`, `custom.rs`, `runtime_router.rs:555`                                                                                                             |
| MCP servers         | stdio servers get `secret_refs` as environment variables. HTTP servers get one fixed `bearer_token`. No custom headers, no OAuth, no older SSE transport                                                                                                                                                                                           | `connectors/mcp.rs:165-210`                                                                                                                                                                                             |
| Relay               | Join code to an HMAC-signed token. The enrolled token is stored **in the DB, not the keychain**                                                                                                                                                                                                                                                    | `relay/mod.rs:597-641`, `relay/server.rs`                                                                                                                                                                               |
| Browser pilots      | A persistent Chromium profile per agent and pilot holds login cookies. Secrets come from the keychain by any name                                                                                                                                                                                                                                  | `pilots/browser.rs`                                                                                                                                                                                                     |
| Other keychain uses | Webhook secrets (`nandi.trigger.<id>`), marketplace sources (`nandi.marketplace.<hash>`), remote credentials (`nandi.remote.credential`)                                                                                                                                                                                                           | `triggers/webhook.rs`, `marketplace/mod.rs`, `remote/credentials.rs`                                                                                                                                                    |
| Frontend            | **Three device-code UIs, each with its own polling**: connector setup, GitHub sign-in and AWS SSO. API-key forms in three places. Separate status types: `GithubSignIn`, `CredentialStatus`, `HarnessHealth.auth`, `LlmKeyStatus`, connector setup hints                                                                                           | `ConnectorSetup.tsx`, `useConnectors.ts`, `GithubSignInPanel.tsx`, `useGithubSignIn.ts`, `AwsSsoSetupDialog.tsx`, `SsoLoginDialog.tsx`, `useAwsAuth.ts`, `LlmKeysSection.tsx`, `ProviderKeyForm.tsx`, `Step2_Model.tsx` |

## Target design

A new `services/auth/` module that owns every credential. Features ask it for a credential and never touch the keychain or run a flow themselves.

```
feature (MCP connector, GitHub, AWS, LLM, relay, pilot, runtime)
  └─ auth::Credentials::get(&CredentialRef) -> Credential   // ready to use, refreshed
       ├─ Store      keychain only, one naming scheme, values never read back to the UI
       ├─ Methods    ApiKey · Bearer · Header(name) · Basic · OAuth(PKCE | device | client credentials) · CliProfile(aws, gh) · HostLogin(claude)
       ├─ Flows      one PKCE engine + one device-flow engine (shared by GitHub, AWS SSO, MCP, Entra)
       ├─ Refresh    one refresher: renews before expiry; on failure the status becomes NeedsSignIn
       └─ Status     one type: Ready | Expiring | NeedsSignIn | NeedsSetup(hint) | Error
```

- **One naming scheme:** `nandi.<owner-kind>.<owner-id>.<name>`, for example `nandi.connector.linear.oauth`, `nandi.llm.deepseek`, `nandi.relay.token`. Nothing has been released, so rename directly: no migration and no aliases (see the no-legacy rule).
- **One status type and one event** (`auth-status-changed`), so every screen shows Sign in, Reconnect and Missing the same way.
- **One set of UI components:** `<SignInFlow>` (shows a device code or "Waiting for browser…", polling built in), `<SecretField>` (set, replace or clear; never shows the value) and `<AuthStatusBadge>`. They replace the three device-code UIs and the three key forms.
- **Domain/repo split holds:** files carry only `secret_refs` and auth _declarations_ (method, scopes, issuer). Tokens live only in the keychain.
- Credentials never enter the chat, the repo or a container image. A container gets only a short-lived value, through the environment or the egress proxy, as it does today.

## Phase 1: Build the layer and move existing features onto it (about 1.5 weeks)

- [ ] `services/auth/`: the `Store`, `Methods`, `Flows`, `Refresh` and `Status` pieces above. `secrets.rs` becomes its private backend.
- [ ] Make one device-flow engine and one PKCE engine from `oauth.rs`. Delete the SSO OIDC copy in `aws_auth/setup.rs` and use the shared engine.
- [ ] Add token refresh, which doesn't exist today.
- [ ] Move each feature onto the layer:
  - [ ] GitHub: one resolver (pasted token → app sign-in → `gh`) used by the connector, marketplace, remote and team
  - [ ] AWS: one resolver used by Bedrock, egress and the connectors
  - [ ] LLM keys and trial
  - [ ] relay token: move it from the DB to the keychain
  - [ ] webhook, marketplace and remote secrets
  - [ ] browser pilot secrets
- [ ] Rename every keychain entry to the one scheme.
- [ ] Frontend: one status type in bindings, plus `<SignInFlow>`, `<SecretField>` and `<AuthStatusBadge>`. Replace the GitHub, AWS SSO, connector and LLM-key UIs with them.
- [ ] Docs: `docs/developer/auth.md` (how a feature asks for a credential). Add a rule to `AGENTS.md`: "No feature talks to the keychain or runs its own sign-in flow." Add an ast-grep or clippy check that `secrets::` is used only inside `services/auth/`.

## Phase 2: MCP servers over HTTP with standard auth (about 1 week)

- [ ] Custom headers from the keychain (`X-API-Key`, Basic, etc.), declared in `config.headers` with values in `secret_refs`.
- [ ] **The MCP authorization spec:** handle a 401 with `WWW-Authenticate`; discover the protected resource and authorization server (RFC 9728, RFC 8414); register Nandi with a client ID metadata document, else dynamic registration (RFC 7591), else a client ID the user enters; PKCE with `resource` set to the server URL; refresh through the shared refresher.
  - First check whether `rmcp`'s `auth` feature covers discovery and registration; use it inside the auth layer rather than writing our own.
- [ ] OAuth with a client ID and secret the user was given (including the client-credentials grant for service accounts).
- [ ] The older SSE transport, for servers that haven't moved to Streamable HTTP.
- [ ] **Test:** live, with public servers: Atlassian's remote MCP server (Jira, Confluence) and one more with OAuth and dynamic registration (Linear or Notion), one with an API-key header, one with a fixed bearer token. Sign in, list tools, restart and stay signed in, revoke and see **Reconnect**.

**Why this phase matters (owner, 2026-10-30):** "Nandi speeds up the process you already have." A developer's flow is: Jira ticket, design in a design database, architect approval, code, review, CI/CD, release. Nandi should drive every step through the company's own tools, signing in as the user, with no tokens pasted. Jira and Confluence are the first targets. Until this phase ships, use a community stdio Jira server (e.g. `mcp-atlassian`) with an API token in the keychain. The architect's approval also needs requests between people that wait days, not minutes: Wave 1 of the team workflows task.

## Phase 3: Local (stdio) MCP servers that sign in by themselves (about 3 days)

- [ ] A persistent state folder per connector, `~/.nandi/local/connectors/<id>/`, mounted read-write as the container's config home, so its token cache survives restarts. Never put it in the agents repo.
- [ ] Watch the server's stderr for a sign-in URL or device code, and show it through `<SignInFlow>` with an **Open** button.
- [ ] Desktop-app servers (Outlook through COM) keep using `connectors.run_on_host`.
- [ ] **Test:** a `uvx` server that uses the device flow. Sign in once; a restart doesn't ask again.

## Phase 4: Add and use an MCP server without chat (about 3 days)

Today the only ways to add a server are asking Studio or editing `.nandi/connectors.yaml` by hand.

- [ ] **Add MCP server** in Library › Connectors: paste a URL, a command, or a README's `mcpServers` JSON. Show a preview (what runs, which hosts it can reach, which secrets and sign-ins it needs, which tools run without asking), then save through the existing connector save path.
  - Conversion rules: the same as the `connect-a-service` Studio guide. Secret-looking environment values become `secret_refs`, and an `Authorization` header becomes a bearer token or header secret.
- [ ] Setup shows the server's auth method through the shared components: secret fields, header fields, or **Sign in**.
- [ ] **Safe tools** checkboxes in Details (today only Studio or YAML can change `safe_tools`).
- [ ] Add the connector to an agent from the connector's page. Today that only happens in the agent's skill.md or Studio.
- [ ] Update the Studio guide `connect-a-service.md`: drop the "browser sign-in isn't supported" lines.

## Done when

- [ ] Only `services/auth/` uses the keychain or runs a sign-in flow, and a static check enforces it.
- [ ] Any MCP server that uses a key, a header, a bearer token, Basic auth, MCP OAuth, given client credentials or device-code sign-in can be added from Library and used by an agent, with no YAML or chat.
- [ ] Every sign-in in the app looks and behaves the same, and tokens refresh without the user noticing.
- [ ] `pnpm run check:all` passes. `docs/testing/` entries are added or updated: connectors (CP-), sign-in (ON-), runtimes, AWS.

## Open questions for the owner

- **Client ID:** should Nandi publish a client ID metadata document (needs a stable `https://` URL we host), or rely on dynamic registration only?
- **Company admins:** should they be able to pre-approve or block MCP servers and auth methods? This would tie into `task-x-company-governance.md`.
- **mTLS** (client certificates): rare for MCP. Leave it out unless a customer needs it.
