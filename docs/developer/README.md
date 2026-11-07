# Developer Documentation

Technical documentation for building and extending this app. These docs describe established patterns and are intended for both human developers and AI coding agents.

## Architecture & Patterns

| Document                                      | Description                                             |
| --------------------------------------------- | ------------------------------------------------------- |
| [Architecture Guide](./architecture-guide.md) | High-level overview, mental models, system architecture |
| [Rust Architecture](./rust-architecture.md)   | Rust module organization and patterns                   |
| [State Management](./state-management.md)     | Three-layer state onion, Zustand, TanStack Query        |
| [Error Handling](./error-handling.md)         | Error propagation, user feedback, retry patterns        |

## Core Systems

| Document                                                                                  | Description                                                                |
| ----------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| [Agent Runtime](./agent-runtime.md)                                                       | Orchestrators, SQLite state, events, snapshots                             |
| [Live UI](./live-ui.md)                                                                   | File watchers, `live://changed`, who changed what, Refresh and F5          |
| [E2E Checklist](./e2e-checklist.md)                                                       | Manual full-loop test with DeepSeek and a container                        |
| [Sandbox and Policy Test Checklist](./sandbox-test-checklist.md)                          | Containers, approvals, the agents' network, policies                       |
| [Egress Proxy](./egress.md)                                                               | LLM/AWS/HTTP proxy, skill.md capabilities, STS                             |
| [Workflows](./workflows.md)                                                               | YAML pipelines of agents, executor, approvals                              |
| [Triggers](./triggers.md)                                                                 | Cron, file, webhook, device, and event triggers                            |
| [Transports](./transports.md)                                                             | Services layer, Tauri IPC + HTTP/WebSocket, web auth                       |
| [Relay Testing](https://github.com/vaultdbai/nandi-relay/blob/main/docs/relay-testing.md) | Running a relay, NANDI_HOME, one-machine and LAN tests (nandi-relay repo)  |
| [Relay Defaults](./relay-defaults.md)                                                     | Relay URL precedence, the LAN prefill, public plan                         |
| [GitHub Auth](./github-auth.md)                                                           | Device-flow GitHub sign-in: one token for every github.com repository      |
| [Email](./email.md)                                                                       | Outlook and Gmail read-only for the Chief of Staff's notes; token refresh  |
| [S3 Backup](./s3-backup.md)                                                               | Folder backups to S3 Deep Archive, AWS setup tools, derived triggers       |
| [Entra ID Setup](./entra-setup.md)                                                        | v2, not in the app: Microsoft sign-in, app registration, Entra-group teams |
| [Marketplace](./marketplace.md)                                                           | Registries, installing, forking, updating agents                           |
| [Pilots](./pilots.md)                                                                     | Host-side OS control, per-agent consent, revoking                          |
| [Command System](./command-system.md)                                                     | Unified action dispatch, command registration                              |
| [Keyboard Shortcuts](./keyboard-shortcuts.md)                                             | Global shortcut handling, platform modifiers                               |
| [Menus](./menus.md)                                                                       | Native menu building with i18n                                             |
| [Companion Window](./companion-window.md)                                                 | Second window, cross-window events, ViewOpener                             |
| [Tauri Commands](./tauri-commands.md)                                                     | Type-safe Rust-TypeScript bridge (tauri-specta)                            |
| [Tauri Plugins](./tauri-plugins.md)                                                       | Plugin usage and configuration                                             |

## UI & UX

| Document                                   | Description                                 |
| ------------------------------------------ | ------------------------------------------- |
| [UI Patterns](./ui-patterns.md)            | CSS architecture, palettes, status tokens   |
| [Internationalization](./i18n-patterns.md) | Translation system, RTL support             |
| [Notifications](./notifications.md)        | Toast and native notifications              |
| [Cross-Platform](./cross-platform.md)      | Platform detection, OS-specific adaptations |

## Data & Storage

| Document                                                  | Description                                               |
| --------------------------------------------------------- | --------------------------------------------------------- |
| [Data Model](./data-model.md)                             | Gold copy (repo files) vs SQLite projection               |
| [Reserved Conventions](./reserved-conventions.md)         | Model routing, inference agents, long-running containers  |
| [Interaction Layer](./interaction-layer.md)               | Ask box, Studio split view, companion states, Work view   |
| [Network and Reassignment](./network-and-reassignment.md) | Trust, task reassignment, manual steps, `.nandi/llm.yaml` |
| [Harness Container](./harness-container.md)               | The harness image, session containers, LiteLLM gateway    |
| [Runtime Routing](./runtime-routing.md)                   | `runtime:` in skill.md, native vs harness runs            |
| [Sign-in](./sign-in.md)                                   | Company identity first, keys last; Claude Code for LLMs   |
| [Domain Model](./domain-model.md)                         | Entities, value objects, aggregate, repos                 |
| [Data Persistence](./data-persistence.md)                 | File storage patterns, atomic writes, SQLite              |
| [External APIs](./external-apis.md)                       | HTTP API calls, authentication, caching                   |

## Quality & Tooling

| Document                                              | Description                                             |
| ----------------------------------------------------- | ------------------------------------------------------- |
| [Feature Test Catalog](../testing/README.md)          | Every shipped feature with test steps; keep it current  |
| [Static Analysis](./static-analysis.md)               | ESLint, Prettier, ast-grep, knip, jscpd, React Compiler |
| [Writing ast-grep Rules](./writing-ast-grep-rules.md) | AI reference for creating custom rules                  |
| [Testing](./testing.md)                               | Test patterns, Tauri mocking                            |
| [Bundle Optimization](./bundle-optimization.md)       | Bundle size management                                  |
| [Logging](./logging.md)                               | Rust and TypeScript logging                             |
| [Writing Docs](./writing-docs.md)                     | Guide for creating and maintaining these docs           |

## Release & Distribution

| Document                  | Description                            |
| ------------------------- | -------------------------------------- |
| [Releases](./releases.md) | Release process, signing, auto-updates |

---

**Updating these docs:** When adding new patterns or systems, update the relevant doc file and add a link here if creating a new document.
