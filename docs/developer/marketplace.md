# Marketplace

The marketplace distributes agents and workflows through shared git repos. It is a layer on the [gold copy](./data-model.md), not a separate store: an install fetches someone else's repo, writes its files into a subfolder of `~/.nandi/repo`, and re-scans. From that point an installed agent loads, projects, and runs like any other agent. The one difference is that it's **read-only until the user forks it**.

Code: `src-tauri/src/services/marketplace/`, `services/repos/{agent_locator,vendor_repo}.rs`, `services/domain/installed.rs`.

## Signatures

Signing is optional for publishers, but every first-party release is signed. A publisher has an Ed25519 key pair, and the public key goes in the registry's `publishers/<handle>.json`. For each release they sign a statement that names exactly one release, and the entry carries both the signature and the commit it names:

```text
nandi-signature-v1\n<agent|pack>\n<id>\n<version>\n<40-char commit SHA>\n
```

```json
{
  "id": "research-pack",
  "latest_version": "1.0.0",
  "commit": "5f01e620…",
  "signature": "ed25519:…"
}
```

The chain of trust runs: publisher key → signature → commit → files. `git tag -s` uses GPG/SSH keys, which is why the publisher signs this statement instead.

| Where      | What happens                                                                                                                                                                               |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Listing    | `verify_signatures` fetches each publisher's key (cached for 1 h) and sets `verified`. This drives the Verified filter and badge, and needs no download beyond `publishers/<handle>.json`. |
| Install    | If the entry names a `commit`, the tag must still resolve to it. Otherwise the install is refused ("may have been tampered with"). `signature_verified` is recorded in `installed.yaml`.   |
| Update     | Verified only if the registry's signed entry is for exactly the commit being applied.                                                                                                      |
| Unverified | Still installable. The UI shows "Publisher not verified" and asks for confirmation first. The agent is read-only and needs first-use consent either way.                                   |

**Publishing a signed release:**

```bash
nandi keygen                     # once: ~/.nandi/keys/publisher.key, prints the public key
nandi sign --kind pack --id research-pack --release 1.0.0 \
          --repo github.com/vaultdbai/nandi-research-pack   # resolves v1.0.0's commit
# paste the printed "commit" and "signature" into packs/research-pack.json, rebuild the index
```

The first-party key lives at `~/.nandi/keys/publisher.key` on the machine that signed v1.0.0. Losing it means publishing a new public key, and every entry has to be re-signed.

## Publishing

**Publish** sits in the agent view header for the user's own agents (`PublishDialog` → `publish_agent`, `services/marketplace/publish.rs`). It runs these steps:

1. **Backed-up release.** The agent's folder (`.nandi/agents/<id>`) must have no uncommitted changes, and `HEAD` must be on a branch of `origin` (Back up now pushes it to the user's own `nandi/<slug>` branch). Otherwise the user gets "Use Back up now in Settings › Backup, then publish". The release is that commit.
2. **Tag.** `<id>-v<version>` is created in the user's agents repo through the API. Prefixed tags let one repo hold many agents, and update checks and version history match tags by prefix (`manifest::split_tag`). Reusing a version for a different commit is refused.
3. **Entry.** Built from skill.md: name, description, capabilities, `ref`, `commit`, `min_nandi_version` = this app's version, optional tags, and a `readme_url` if the folder has a README. It's signed when `~/.nandi/keys/publisher.key` exists.
4. **Pull request.** The flow creates the branch `nandi/publish-<tag>`, writes `agents/<id>.json`, upserts the entry into `index.json`, and adds `publishers/<login>.json` with the public key if it's missing. When the user can't push to the registry (the public one), it forks the registry first and opens the PR from the fork. On a community registry the user can push to, the PR is merged right away and the index cache is refreshed.

`github.rs` holds the REST calls (refs, Contents API, forks, pulls). Tests run the whole flow against a fake GitHub.

## Content

First-party content lives in its own public repos, not in this one. The exception is the built-in agent Chief of Staff, which ships inside the app (`services/builtin_agents.rs`) as ordinary local agents; see `docs/developer/a2a-delegation.md`. The app only knows the registry URL (`DEFAULT_REGISTRY`) and reads its `index.json` from `HEAD`, so new releases reach users without an app update.

| Repo                                                                                  | Contents                                                                 |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| [vaultdbai/nandi-registry](https://github.com/vaultdbai/nandi-registry)               | `index.json`, `agents/`, `packs/`, `publishers/`, `tools/build_index.py` |
| [vaultdbai/nandi-dev-pack](https://github.com/vaultdbai/nandi-dev-pack)               | `dev-pack`: pr-reviewer, aws-ops-analyst                                 |
| [vaultdbai/nandi-personal-backup](https://github.com/vaultdbai/nandi-personal-backup) | `personal-backup`: backup-keeper, workflow set-up-s3-backup              |

The registry lists `dev-pack` (2026-10-24) and `personal-backup` (2026-10-30), both official starter packs. `tools/build_index.py` indexes every folder with `.nandi/manifest.yaml` next to the registry, so build a release in a workspace holding only the published packs. The earlier packs (research, engineering, content, analytics, personal and eight more) were for trying things out. Their entries are kept in the registry's `archive/`, outside the index, and their repos still exist; moving an entry back to `agents/` / `packs/` relists it. New first-party agents are built and listed one at a time.

**Releasing a pack.**

1. In the pack repo, bump `version` in `.nandi/manifest.yaml`, tag `vX.Y.Z`, and push the tag.
2. Clone the pack repos next to `nandi-registry` (folder `nandi-<id>` or `nandi-<id>`) and run `python tools/build_index.py` there. It validates every pack and regenerates the derived fields, including `latest_version` and `ref`.
3. Sign the pack and each of its agents at the tag's commit (`nandi sign --kind pack|agent --id <id> --release X.Y.Z --commit <sha> [--key <path>]`), put each `commit` and `signature` into `packs/<id>.json` and `agents/<id>.json`, run `build_index.py` again, then push the registry.

Installed users then see the new version under **Check for updates**.

**Testing against the live registry.** `cargo test --lib live_starter_packs -- --ignored` fetches the default registry and installs every pack through the real installer. The regular suite uses local git fixtures and never touches the network.

Each agent has its own working folder (`~/.nandi/local/work/<agent-id>/`). Workflows therefore pass data between agents only through `{{step.output}}`, never through files.

## Layout

```
~/.nandi/repo/.nandi/
  agents/
    my-agent/                                  the user's own (editable)
    marketplace/
      packs/research-pack/                     a whole pack
        .nandi/manifest.yaml
        agents/arxiv-researcher/skill.md …
        workflows/literature-review.yaml       global-scope, read-only
      alice/arxiv-researcher/                  one agent from alice's repo
  installed.yaml                               every vendored source
```

Agent IDs stay flat and unique across the repo. `agent_locator` finds an agent's folder wherever it lives. A local folder wins over a vendored folder with the same ID, and the shadowed copy is reported as a duplicate. `marketplace` is reserved as an agent ID and `packs` as a publisher handle.

| Who writes             | What                                                     |
| ---------------------- | -------------------------------------------------------- |
| `VendorRepository`     | vendored folders, `installed.yaml` (the only writer)     |
| `AgentRepository`      | refuses `save`/`delete` of a vendored agent (`ReadOnly`) |
| `WorkflowRepository`   | refuses writes to pack and vendored-agent workflows      |
| `agents::delete_agent` | refuses marketplace agents: uninstall them instead       |

## Registries

A registry is a git repo, or any static host, with this layout:

```
index.json            { "schema_version": 1, "agents": [...], "packs": [...] }
agents/<id>.json      one entry (same fields as in the index, plus readme/icon URLs)
packs/<id>.json
publishers/<handle>.json
```

`settings.yaml` lists the registries under `marketplace.registries`. If the key is absent, only the default public registry (`DEFAULT_REGISTRY`, `github.com/vaultdbai/nandi-registry`) is used. A private registry's token is stored in the keyring, and `auth:` holds only its reference. `auth: nandi.github.token` means "use the signed-in GitHub account".

Several registries merge into one list, one entry per ID (`registry::merge`, the same rule for agents, packs and connectors). The default registry wins every ID it lists, whatever version another registry claims, so an added registry can't shadow a first-party entry. Among the others the highest version wins; a tie goes to the earlier registry. Only the default registry's entries keep the `official` tag, which puts a verified entry first and ticks it in onboarding.

`RegistryClient::fetch_index` caches each index in `~/.nandi/local/cache/registry-<hash>.json` along with its ETag:

| Situation              | Behaviour                                                        |
| ---------------------- | ---------------------------------------------------------------- |
| within TTL             | cache, no request (public 1 h, community 15 min, personal 5 min) |
| after TTL / `refresh`  | conditional GET; a `304` reuses the cache                        |
| rate-limited / offline | cache + `warning`; error only if there's no cache                |
| bad entry in the index | skipped with a log line; the rest of the index still loads       |

`RegistryList::merge_agents` / `merge_packs` deduplicate entries by ID. The highest `latest_version` wins, and a tie goes to the earlier registry.

Entries are validated as they're parsed (`manifest.rs`). IDs, publisher handles, relative `path`s (no `..`), git refs (no option-like names), versions, and asset URLs (`https://` or relative) all go through domain rules, so a hostile registry can't escape the checkout or the repo.

## Installing

```
install_pack(manifest):                       install_agent(manifest):
  min_nandi_version <= this app, else refuse     (same)
  ls-remote → resolve ref → commit              (same)
  fetch that ref into ~/.nandi/local/cache/checkouts/<uuid> (bare, outside the repo)
  read .nandi/manifest.yaml, re-check min_nandi   read files under manifest.path
  keep allowlisted files only                   keep allowlisted files only
  parse every agent + workflow (domain types)   parse the agent + its workflows
  refuse if any agent/workflow ID is taken      (same)
  write marketplace/packs/<id>/ in one rename   write marketplace/<publisher>/<id>/
  installed.yaml entry (commit, tree_hash)      (same)
  RepoStore::rescan_all(User)                   (same)
```

Files are read straight from git objects; nothing is checked out. A symlink in the source is an error and submodules are skipped. Only `skill.md`, `instructions.md`, `README.md`, `icon.{svg,png}`, `LICENSE*`, `workflows/*.yaml`, and (for packs) `.nandi/manifest.yaml` are copied. Anything else in a source repo, such as scripts, is left behind.

**Installer, watcher and projection.** The installer writes files. It never touches SQLite. A folder is staged under a hidden `.<name>.nandi-tmp-<uuid>` (which the watcher and locator ignore), then renamed into place. The watcher sees one folder appear and re-scans. The installer also calls `rescan_all` itself, so the projection is current when the command returns. Whichever runs second finds equal hashes and does nothing. A folder cloned in by hand is picked up the same way, without an install.

## Forking

`fork_agent(installed_id, new_id, new_name)` moves an agent into the user's own agents:

1. Copy the vendored folder to `.nandi/agents/<new_id>/`.
2. Rewrite `skill.md` through `Agent::forked`: new ID and name, and the source metadata keys (`source`, `publisher`, `version`, …) dropped.
3. Point the agent's own workflows at the new ID (`Workflow::rename_agent`).
4. Trash the vendored copy. Remove its `installed.yaml` entry, or for a pack agent add it to the pack's `forked_out` and record the pack's new `tree_hash`.
5. Re-scan. The result lists anything that still names the original agent, such as a pack workflow.

## Updates

Updates are never automatic. `check_for_updates` compares each install with its source's newest release tag (`vX.Y.Z`; for a branch install, the branch head) and only reports what it finds. `apply_update(id, ref)` refuses if the folder's `tree_hash` no longer matches the install. To list exactly which files changed, it re-fetches the installed commit and compares. Forking is the way to keep local changes.

## Events

| Topic                            | Payload                                          |
| -------------------------------- | ------------------------------------------------ |
| `marketplace://installed`        | `kind, id, source, version`                      |
| `marketplace://uninstalled`      | `kind, id`                                       |
| `marketplace://forked`           | `sourceId, newId`                                |
| `marketplace://update_available` | `kind, id, currentVersion, latestVersion`        |
| `marketplace://update_applied`   | `kind, id, fromVersion, toVersion`               |
| `marketplace://error`            | `action, id, error`                              |
| `marketplace://progress`         | `kind, id, receivedObjects, totalObjects` (≤5/s) |

The re-scan also emits the usual `repo://agent-*` and `agent://agents_changed` events.

## Commands and routes

| Tauri command                    | HTTP                                      |
| -------------------------------- | ----------------------------------------- |
| `list_available_packs`           | `POST /api/rpc/list_marketplace_packs`    |
| `list_marketplace_packs(filter)` | `POST /api/rpc/list_marketplace_packs`    |
| `list_available_agents(filter)`  | `POST /api/rpc/list_available_agents`     |
| `get_agent_detail(agent_id)`     | `POST /api/rpc/get_agent_detail`          |
| `install_pack(pack_id)`          | `POST /api/rpc/install_pack`              |
| `install_agent(agent_id)`        | `POST /api/rpc/install_agent`             |
| `list_installed`                 | `POST /api/rpc/list_installed`            |
| `uninstall_installed(kind, id)`  | `POST /api/rpc/uninstall_installed`       |
| `fork_installed_agent(…)`        | `POST /api/rpc/fork_installed_agent`      |
| `check_for_updates`              | `POST /api/rpc/check_for_updates`         |
| `apply_update(id, new_ref)`      | `POST /api/rpc/apply_update`              |
| `list_registries`                | `POST /api/rpc/list_registries`           |
| `add_registry(input)`            | `POST /api/rpc/add_registry`              |
| `remove_registry(url)`           | `POST /api/rpc/remove_registry` (encoded) |

## Frontend

| Piece                        | Role                                                                                           |
| ---------------------------- | ---------------------------------------------------------------------------------------------- |
| `hooks/useMarketplace.ts`    | TanStack Query hooks; every key starts with `['marketplace']`                                  |
| `lib/marketplace-actions.ts` | `installPacks` (one after another, reporting each pack's status)                               |
| `store/marketplace-store.ts` | live fetch progress from `marketplace://progress`, keyed `pack:<id>` / `agent:<id>`            |
| `lib/agent-sync.ts`          | `marketplace://installed`/`uninstalled`/`forked`/`update_applied` invalidate `['marketplace']` |
| `components/marketplace/`    | `PackCard`, `CapabilityBadges`, `MarketplaceIcon` (shared by onboarding and the browser)       |

**Onboarding.** On first launch, picking an agents repo (the `needs-repo` auth phase) leads to `StarterPacksView` (`{ kind: 'starter-packs' }`) instead of the Dashboard. Choosing "Change agents repo" later goes straight back to the Dashboard. The user can select any number of packs: already-installed packs are hidden, and packs that need a newer Nandi can't be selected. Packs install in sequence with per-pack progress, and the Dashboard opens once all of them succeed. "I'll set up manually" skips to an empty Dashboard.

**Browser.** The Library page (`components/library/`, `{ kind: 'library', tab?, filter?, capability?, item? }`) replaced `MarketplaceView`; the old route redirects to Library › Browse. Marketplace agents and packs, the workflows packs bring, built-in tools, connectors and pilots are all `LibraryItem`s from the pure `library-model.ts` (permissions line, one status pill, one action), filtered client-side by kind, search words and Can do (tags until manifests declare `provides`). Clicking a card opens `LibraryDrawer` (`item` in the view, so Back closes it). Install goes through `InstallConsentDialog`, which lists the access (`capabilityNotice`, anything undeclared is denied by the runtime and egress proxy), and then `installAndAllow` (`library-actions.ts`) installs and grants consent for each installed agent's required categories. Check for update, Fork and Uninstall are in the drawer's ⋯ menu. Sources live in Settings › Sources. There are no star ratings.

**Installed tab.** It lists every vendored pack and agent: source, version, install time, and a _Modified_ badge when the files no longer match `tree_hash`. _Check for updates_ is the only thing that looks for new versions. Results stay in component state, and _Update_ / _Update all_ apply them one at a time (a modified install can't be updated). The row menu forks an agent (for a pack, one of its agents) or uninstalls after a confirmation.

**Agent view.** `SharedAgentBanner` sits on every marketplace agent. It says the agent is read-only, offers _Fork_, and, until the agent has consent, shows _Review and allow_, which opens `ConsentDialog`. That dialog lists each requested category (tools, network hosts, AWS, pilots, connectors, shared folders with their access as `docs:read` or `docs:write`, and taking tasks from others) with its items. Each one must be ticked before _Allow_. Approving records the items the dialog showed, nothing more: an update that lands while it is open stays unapproved. A later version that widens any category (a new connector, a new folder, read made write, `accepts_delegation`) needs approval again; narrowing a folder to read keeps it. The backend enforces the same rule in `chat::submit`, so a workflow or trigger can't run the agent before the user has approved it either (see `services/marketplace/consent.rs`).

**Registries tab.** It lists each configured registry (kind, default / private badges, agent and pack counts, last fetch, and any warning such as a stale cache or refused access) with _Remove_, which asks for confirmation; installed agents stay installed. _Add registry_ takes a URL, a kind, and either a token (stored in the keyring) or _Use my GitHub sign-in_. The core fetches the index once before saving, so a wrong URL fails right away.

**Community registry.** _Create my community registry_ (`create_community_registry`, `services/marketplace/community.rs`) uses the signed-in token. It creates a **private** `nandi-community-<login>` repo and seeds it through the Contents API with an empty `index.json`, `publishers/<login>.json`, `agents/`, `packs/`, and a README explaining how to join and share. It then adds the repo as a community registry read with the sign-in. Running it again reuses the repo and only fills in missing files. The result includes the URL to share and the GitHub page where the owner invites collaborators.

Icons load straight from their `https://` registry URLs (the CSP allows `img-src https:`) and the webview caches them. They're monochrome and inverted in dark mode, and a generic glyph stands in when an icon is missing or fails to load.

## Not built (by design)

Payments, ratings, install analytics, auto-updates, custom hosting. GitHub is the CDN.
