# GitHub Security Check

With the GitHub sign-in (`services/auth/github.rs`), Nandi checks the account and the orgs the user owns for the weak settings attackers use, and keeps one `check` to-do per finding ([todos.md](./todos.md)). Needs relay: no.

It serves **trust** (weak sign-in and branch settings found) and **state** (each finding is one to-do, kept in line with GitHub on every check).

## Rules

- **Read-only, host-side.** `GET` calls only, settings and metadata only: never secret values, never repository contents. It is Nandi's own code on the host, like the email connector; nothing runs in a container.
- **The token goes only to `api.github.com`.** `reader::API_URL` is the only base in a build. `GitHubApi::send_with_headers` refuses a path that doesn't start with `/`, and only names that pass `is_github_name` / `is_branch_name` go into a path.
- **Bounded.** At most `reader::MAX_CALLS` (150) calls and `MAX_REPOS_PER_ORG` (30) public repositories per org per check. A rate limit (`429`, or `403` with `x-ratelimit-remaining: 0`) or a `401` stops reading.
- **Never guessed.** A field GitHub didn't show makes that check `Unknown`, and an unknown check changes no to-do.
- **Never blocks.** `check_soon` runs it in the background right after a sign-in (`auth::github::poll`); Settings › Account has **Check GitHub security** and the last check's time (`config.json` `githubSecurity`).

## Pieces

| Piece                                    | Where                                                                       |
| ---------------------------------------- | --------------------------------------------------------------------------- |
| Rules, the to-do plan (pure, tested)     | `services/domain/github_security.rs`                                        |
| The reader (calls, budget, rate limit)   | `services/github_security/reader.rs`                                        |
| The service (`check`, `status`, `apply`) | `services/github_security/mod.rs`                                           |
| Commands                                 | `get_github_security_status`, `check_github_security` (`rpc/identity.rs`)   |
| The button and line                      | `src/components/github/GithubSecurityCheck.tsx`, `github-security-model.ts` |

## Checks

| Source ref                                 | Fails when                                                                                                        | Read from                                                   |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `github:user:2fa`                          | `two_factor_authentication` is false                                                                              | `GET /user`                                                 |
| `github:repo:<owner>/<repo>:public-backup` | the Backup remote (`config.json` `remote.url`) is a public github.com repository                                  | `GET /repos/{o}/{r}`                                        |
| `github:org:<org>:2fa`                     | `two_factor_requirement_enabled` is false                                                                         | `GET /orgs/{org}` (orgs where the membership role is admin) |
| `github:org:<org>:secret-scanning`         | a public repo has secret scanning or push protection disabled                                                     | `GET /orgs/{org}/repos?type=public` `security_and_analysis` |
| `github:org:<org>:branch-protection`       | a public repo's default branch has no classic protection and no rules with both `non_fast_forward` and `deletion` | `GET /repos/{o}/{r}/branches/{b}`, `/rules/branches/{b}`    |
| `github:org:<org>:unreadable`              | the org hid its settings with the right scopes: it limits third-party apps                                        | `GET /orgs/{org}`                                           |
| `github:scope:read`                        | the sign-in lacks `read:org` or `read:user` (`X-OAuth-Scopes`), or memberships are hidden                         | `GET /user`, `/user/memberships/orgs`                       |

Refs are lowercase, so a re-check matches. Archived repositories are skipped.

## To-dos

`domain::github_security::plan` turns the checks into steps, applied with `gateway::todos` as Nandi (`Actor::Agent(DEFAULT_AGENT)`), source `security_check`:

| Check                  | To-do there                 | Step                                              |
| ---------------------- | --------------------------- | ------------------------------------------------- |
| fails                  | none                        | `add` (kind `check`, the sentence with its link)  |
| fails                  | open, or closed by the user | nothing                                           |
| fails                  | closed by Nandi as fixed    | `reopen`                                          |
| passes                 | open                        | `done` with "Fixed: checked again on YYYY-MM-DD"  |
| unknown                | any                         | nothing                                           |
| not named, ref settled | open                        | `done` (an org no longer owned, a changed remote) |

`settled` lists the ref prefixes a check saw in full: `github:repo:` always, `github:org:` once the memberships were listed. Every step writes its Decisions entry like any to-do change.

## Scopes (decision, 2026-11-20)

`SCOPES` is `repo`, `read:org`, `read:user`. The two read scopes are read-only: GitHub needs `read:org` to list org memberships and show an org's two-factor setting, and `read:user` for the account's private fields. A sign-in from before keeps working for everything else: the check sees the missing scopes and adds one to-do to sign in again.
