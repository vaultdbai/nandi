# Task: Shared folders, read or read and write, given to agents one by one

**From:** owner feedback 2026-10-20: "the connector list must let the user add multiple folders with read permission, and choose write there too; the user should be able to update an agent, or ask Ask to, to mount specific folders from that list."
**Size:** M.

## Today

- The `filesystem` connector (`services/connectors/filesystem.rs`) shares **one** host folder (`config.path`), read and write; `write_file` is marked sensitive (asks for approval).
- An agent gets its tools by listing it in skill.md: `connectors: [filesystem]` (`connectors::declared_connectors`). It gets the whole folder or nothing.
- Since 2026-10-20 Ask can install connectors and save agents (`nandi_save_artifact`, `nandi_list_connectors`), and `connectors` is a documented skill.md key.

## Design

- **The list** (`connectors.yaml`, filesystem connector config):

  ```yaml
  - id: filesystem
    kind: filesystem
    config:
      folders:
        - id: docs # short name agents use
          path: ~/Documents/agent-share
          access: read # read | write (write includes read)
        - id: reports
          path: ~/Reports
          access: write
  ```

  The old single `path` reads as one folder `{id: shared, access: write}` (migrated on save, so old files keep working).

- **Per agent** (skill.md), by folder ID:

  ```yaml
  connectors:
    - id: filesystem
      folders: [docs, reports] # omit: none (not "all")
  ```

  `declared_connectors` already accepts `{id: …}` items; add the `folders` list. An agent's access to a folder is the folder's access from the list, and can be narrowed to `read` per agent (`- {folder: reports, access: read}`) but never widened.

- **Tools**: paths become `<folder-id>/<relative path>`; `list_dir` at the root lists the agent's folders. `write_file` on a read folder is refused before any approval card. Containment checks per folder as today (no `..`, no symlinks out).
- **Container agents**: folders are bind-mounted at `/agent/folders/<id>` (`:ro` for read) in the native container and in CLI runtime containers. **Decided (2026-10-21):** both routes, one source. The connector's tools serve native agents on the host; the mounts serve `shell` and the CLI runtimes, which have no connector tools. Both come from `connectors::agent_folders`, so they can't disagree, and the system prompt lists the mounts.
- **Old setups**: an agent without `folders` gets only `shared` (the folder from the old single `path`), so nothing set up before breaks and a folder added later is never given to anyone silently.

## Work

- [x] Domain + config parsing for `folders` (ids unique, paths absolute after `~`, access enum), with the `path` migration. Tests.
- [x] `FilesystemConnector` over many roots; the agent's set comes from its skill.md item (`Connector::call_tool_for`). Tests for read-only refusal and containment.
- [x] Library › Connectors › Filesystem › Setup: add (native folder picker), short name, Read only / Read and write, remove.
- [x] Show which agents use each folder (`list_folder_users`), and ask before removing one in use.
- [x] Agent page: "Folders" picker listing the shared folders with a per-agent read/write choice; saves skill.md through the agent repository.
- [x] Ask: `nandi_list_connectors` shows the folders; the build-an-agent guide says how to give an agent folders; `nandi_check_artifact` errors on a folder ID that isn't in the list.
- [x] i18n in en, fr, ar, ja; `docs/developer/` connector doc and the user guide.

## Acceptance

- [ ] Add two folders (one read, one read & write), give an agent only the read one: it can list and read it, can't write it, and can't see the other.
- [ ] "Give the report agent write access to Reports" in Ask shows one approval card and the agent's skill.md changes.
