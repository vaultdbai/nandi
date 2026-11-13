# Entra ID Setup

> **Planned for v2. Not in the app today.** v1 has no sign-in: the first run reads the local identity in `NANDI.md`, and teams can be any git URL. This page is kept as the design for v2. See `docs/tasks-todo/task-x-v2-sign-in.md`.

Nandi can sign in with Microsoft Entra ID (formerly Azure AD) and build teams from Entra groups. Sign-in is the OAuth 2.0 Device Authorization Grant (RFC 8628), the same flow as [GitHub sign-in](./github-auth.md): Nandi shows a code, opens `https://microsoft.com/devicelogin`, and polls until you approve. No redirect URI or client secret is involved.

Microsoft sign-in is additional to GitHub. GitHub still holds the agents repo; signing in with Microsoft from the Login screen opens the app with local agents, the way "Continue without GitHub" does, and Entra groups can then back teams.

## Register the app (one time, per tenant)

An Entra admin (or anyone allowed to register apps) does this once:

1. In the [Entra admin center](https://entra.microsoft.com), open your tenant, then **App registrations → New registration**.
2. **Name:** `Nandi`.
3. **Supported account types:** _Accounts in this organizational directory only_.
4. **Redirect URI:** leave empty. Click **Register**.
5. **Authentication → Advanced settings → Allow public client flows: Yes**, then **Save**. The device code flow requires it; without it Microsoft answers `invalid_client`.
6. **API permissions → Add a permission → Microsoft Graph → Delegated permissions:** add `User.Read` and `GroupMember.Read.All`.
7. Click **Grant admin consent for <tenant>**. `GroupMember.Read.All` needs admin consent; without it, listing groups returns 403 ("An admin must grant consent…").
8. From the app's **Overview**, copy the **Directory (tenant) ID** and the **Application (client) ID**.

## Configure Nandi

Open **Settings → Entra ID** (or **Tools → Entra ID** in the sidebar) and enter:

| Field         | Value                                                                                              |
| ------------- | -------------------------------------------------------------------------------------------------- |
| Tenant ID     | The directory (tenant) GUID, or a verified domain such as `contoso.onmicrosoft.com`                |
| Client ID     | The application (client) ID GUID                                                                   |
| Authority URL | Empty for `https://login.microsoftonline.com`; set it for sovereign clouds (must be `https://...`) |

**Save** writes `entra_tenant_id`, `entra_client_id`, and `entra_authority` (only when not the default) to `~/.nandi/config.json`. Changing the tenant, client ID, or authority signs you out of Microsoft, since the old tokens belong to the old app.

**Test connection** asks Microsoft for a device code with the saved settings. That checks the tenant and client ID (an unknown tenant, an unknown app, or public client flows being off all fail here) without signing in; the unused code just expires.

Until both IDs are set, `start_microsoft_device_flow` returns exactly:

> Entra ID is not configured. Set the tenant ID and client ID in Settings → Entra ID.

### IT-managed installs

Environment variables override the file, and make the form read-only:

| Variable                | Overrides         |
| ----------------------- | ----------------- |
| `NANDI_ENTRA_TENANT_ID` | `entra_tenant_id` |
| `NANDI_ENTRA_CLIENT_ID` | `entra_client_id` |
| `NANDI_ENTRA_AUTHORITY` | `entra_authority` |

`EntraConfigStatus.fromEnv` is true when the tenant or client ID comes from the environment; `set_entra_config` is then refused.

## Sign-in and tokens

- Scopes: `openid profile email User.Read GroupMember.Read.All offline_access`.
- Endpoints: `POST {authority}/{tenant}/oauth2/v2.0/devicecode`, then `POST {authority}/{tenant}/oauth2/v2.0/token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code` until it stops answering `authorization_pending` (`slow_down` adds 5 seconds to the interval).
- On success Nandi calls Graph `GET /v1.0/me` (which validates the token) and shows the `userPrincipalName` (falling back to the ID token's `preferred_username`).
- Tokens go to the OS keychain under `nandi.auth.microsoft.access_token`, `.refresh_token`, and `.id_token`; the access token's expiry (`.expires_at`) and the username (`.username`) sit next to them, so **Sign out** (`logout_microsoft`) clears the whole session. Values longer than one Windows Credential Manager entry allows are split over `<key>.part<N>` entries.
- A stale access token is refreshed with `grant_type=refresh_token`; if Microsoft refuses the refresh token, or Graph answers 401, the session is cleared and the user is asked to sign in again.
- Tokens never reach a client. Clients see `AuthStatus` only.

## Groups and teams

`list_entra_groups` reads `GET /v1.0/me/memberOf` (following `@odata.nextLink`, at most 20 pages, and only back to the Graph host), keeps `#microsoft.graph.group` entries (directory roles and administrative units are dropped), and caches them in `entra_teams_cache`.

In the **Groups I'm a member of** list, each group can be:

- **Mapped to a team** (`set_team_source(team, "entra", groupId)`, which also writes `entra_group_team_map` in `config.json`); choosing _Not mapped_ removes it from the map.
- **Created as a team** (`create_team_from_entra_group`): a Nandi team whose ID is the group name's slug, source `entra/<groupId>`, and registry repo `github.com/<your GitHub user>/<slug>-registry` (just `<slug>-registry` when not signed in to GitHub). Nandi doesn't create that repo.

**Sync from Entra ID** (Teams view, or **Re-sync** on an Entra team) runs `sync_entra_teams`: for each of my groups that maps to a team (the map, else a team already sourced from it, else, with `entra_auto_map_by_name: true`, a team with the same name), it reads `GET /v1.0/groups/{id}/members`, keeps users, and makes them the team's members (UPN, else mail, as the handle; roles and join times of existing members are kept). It never creates teams. A team's registry sync no longer refreshes members from GitHub once it is Entra-backed.

Teams carry `source: { kind: "github" | "entra", identifier }`, persisted in `teams.source_kind` / `source_identifier`; rows from before v9 are GitHub teams.

## Code

| Piece          | Where                                                                           |
| -------------- | ------------------------------------------------------------------------------- |
| Sign-in, Graph | `src-tauri/src/services/auth/microsoft.rs` (mock server in its `mock` module)   |
| Group teams    | `src-tauri/src/services/team/entra.rs`                                          |
| Commands       | `commands/auth.rs` (Microsoft), `commands/entra_config.rs`, `rpc/teams.rs`      |
| Routes         | `src-tauri/src/http/entra.rs`                                                   |
| UI             | `src/components/entra/`, `EntraSetupView`, `EntraPane`, `Login`, `TeamView`     |
| Hooks          | `src/hooks/useEntraAuth.ts` (queries, mutations, and the shared device polling) |

`MicrosoftAuthService` (from `AppState::microsoft_auth()`) holds the HTTP client and, in tests, the login and Graph base URLs of a loopback mock (`set_endpoints`).
