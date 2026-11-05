# GitHub Sign-in

One optional sign-in gives Nandi a GitHub token for every github.com repository it uses. It's the [OAuth Device Flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow) (RFC 8628): Nandi shows a code, opens `https://github.com/login/device`, and polls until the user approves. No redirect server or client secret is involved, so it works in a desktop app and in the browser transport alike.

Identity stays local (`NANDI.md`); this sign-in only supplies a credential.

## Pieces

| Piece                           | Where                                                                                                                   |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Provider, token lookup, status  | `src-tauri/src/services/auth/github.rs`                                                                                 |
| Device flow (shared with email) | `src-tauri/src/services/auth/oauth.rs` (`OAuthFlows`, keyed by keyring entry)                                           |
| Flow in progress                | `AppState::github_sign_in_flows()`                                                                                      |
| Tauri commands                  | `commands/identity.rs`: `get_github_sign_in`, `start_github_sign_in`, `poll_github_sign_in`, `sign_out_github`          |
| HTTP routes                     | `http/identity.rs`: `GET/DELETE /api/github/sign-in`, `POST /api/github/sign-in/start`, `POST /api/github/sign-in/poll` |
| UI                              | `src/components/github/GithubSignInPanel.tsx`, `src/hooks/useGithubSignIn.ts`                                           |

## Where the token is used

| Consumer                     | How                                                                                                               |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| GitHub connector             | `catalog.rs`: a pasted token first, then the sign-in (only when `api_url` is github.com), then `gh auth token`    |
| Backup (`services/remote`)   | `CredentialKind::Github`: the stored credential holds no token; `credential()` fills in the sign-in's when loaded |
| Team registries              | `TeamService::registry_token(url)` → `registry::sync` / `push_head`                                               |
| Capability registries        | `git_sync::clone_or_pull(.., token)` from `CapabilityRegistryService::sync_registry`                              |
| Marketplace sources          | A registry whose `auth` is `nandi.github.token` ("Use my GitHub sign-in") reads through `auth::github::token`     |
| Community source, publishing | `auth::require_token`: the sign-in, else a Backup token credential                                                |

`auth::github::token_for_url(secrets, url)` returns the token only for `https://github.com/...` (and `www.`). SSH addresses (`git@github.com:...`) keep using SSH keys, and other hosts never see it.

## Security rules

- **The token never leaves Rust.** It lives in the OS keychain at `nandi.github.token` (JSON written by `OAuthFlows`, read with `oauth::stored_access_token`). Clients only get `GithubSignIn { signedIn, login }`.
- **Never in files or URLs.** git2 gets it through credential callbacks (`x-access-token` + token), so it never lands in `.git/config`. Backup's keychain credential for `Github` stores no token.
- **github.com only**, per the rule above.
- The login (not a secret) is kept as `github_login` in `~/.nandi/config.json` for display.
- **Sign out** deletes the keychain entry and the login. The app stays authorized on GitHub (Settings › Applications) until the user revokes it there.

## Configuration

The Nandi OAuth App's client ID (`DEFAULT_CLIENT_ID` in `auth/github.rs`) is public and baked in. The app must have **Enable Device Flow** checked. To use another app, set it at build time:

```bash
NANDI_GITHUB_CLIENT_ID=Ov23li... pnpm tauri dev
```

The scope is `repo` (read and write the user's repositories: Backup pushes, team registries, pull request reviews). Organizations that restrict third-party OAuth apps must approve the app once before their private repositories are reachable.

## Manual test

See ST-32 in `docs/testing/shell-onboarding-settings.md`.
