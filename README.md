<p align="center">
  <img src="src/assets/brand/mascot/companion-bison.webp" alt="The Nandi bison, lying down and breathing" width="240" />
</p>

<h1 align="center">Nandi</h1>

<p align="center"><strong>Connect every team through AI agents you can trust.</strong><br />by <a href="https://www.vaultdb.ai">VaultDB.ai</a></p>

A local-first desktop app for managing a team of AI agents. You act as the manager: each agent runs in its own Docker container, is defined by a `skill.md` file, and reports back to you through the **Agent Console**.

> **Status:** 0.1.0, the first release. Download it from [Releases](https://github.com/vaultdbai/nandi/releases/latest) (Windows `-setup.exe`, no admin rights needed, or `.msi` for IT deployment; macOS `.dmg`; Linux `.AppImage`; requires Docker).

## Why it runs on your machine

Model calls are the cost that grows. Running every agent, tool and coding CLI in a hosted service on top of them makes it grow faster: hosted compute is billed per minute, and a team of agents runs all day. Most people already have a desktop or laptop powerful enough to run agents in containers, and it sits idle much of the time. Nandi uses that machine:

- **Cost:** you pay only for model calls. Agents, tools, containers and storage run on hardware you already own. Local models through Ollama are an option for work that doesn't need a hosted model.
- **Your setup:** agents use the tools, packages, registries and settings already on your machine, so there's nothing to rebuild in someone else's cloud.
- **Your data:** files, history and definitions stay on your disk (`~/.nandi`). Only the model calls you allow leave it, through the egress proxy.
- **Safety:** "local" doesn't mean "on the host". Agent code runs only in hardened containers (read-only root, no network except the proxy), and credentials are passed per session, never baked in.

## Features

### Agent Console

A three-panel workspace for supervising agents:

- **Agent list** — every agent with its status (running, needs approval, idle) and current task
- **Conversation** — an activity feed of messages between you and the selected agent, including **approval requests** the agent needs you to approve or reject before it acts (e.g. running a shell command)
- **Context panel** — tabs for the agent's **Workflow**, **Permissions** (what its container may do), **Connectors** (external services), and **Git** state

### Local Docker runtime

Nandi talks to the local Docker daemon via [bollard](https://github.com/fussybeaver/bollard). Currently available:

| Command          | Description                                |
| ---------------- | ------------------------------------------ |
| `listContainers` | Lists all containers (running and stopped) |

## Stack

| Layer    | Technologies                                    |
| -------- | ----------------------------------------------- |
| Frontend | React 19, TypeScript, Vite                      |
| UI       | shadcn/ui, Tailwind CSS v4, Lucide React        |
| State    | Zustand v5, TanStack Query v5                   |
| Backend  | Tauri v2, Rust, bollard (Docker API)            |
| Testing  | Vitest, Testing Library, cargo test             |
| Quality  | ESLint, Prettier, ast-grep, knip, jscpd, clippy |

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org) 20+
- [Rust](https://rustup.rs) (latest stable)
- [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform
- [Docker](https://docs.docker.com/get-docker/) running locally (Docker Desktop on macOS/Windows)

### Run

```bash
pnpm install
pnpm run tauri:dev
```

## Development

| Command                  | What it does                                           |
| ------------------------ | ------------------------------------------------------ |
| `pnpm run tauri:dev`     | Run the desktop app in development mode                |
| `pnpm run test:run`      | Run frontend tests                                     |
| `pnpm run rust:test`     | Run Rust tests                                         |
| `pnpm run rust:bindings` | Regenerate TypeScript bindings for Tauri commands      |
| `pnpm run check:all`     | Full quality gate (types, lint, format, clippy, tests) |

This project uses **pnpm** only.

### Project layout

```
src/
├── pages/                    # Top-level screens (AgentConsole)
├── components/
│   ├── agent-console/        # Agent list, chat, approval cards, context panel
│   ├── layout/               # App chrome (MainWindow, sidebars)
│   └── ui/                   # shadcn/ui components
├── store/                    # Zustand stores
├── lib/                      # Utilities, mock data, generated Tauri bindings
└── types/                    # Shared TypeScript types
src-tauri/src/
├── commands/                 # Tauri commands by domain (docker, preferences, ...)
└── bindings.rs               # tauri-specta command registration
locales/                      # Translations (en, fr, ar)
```

### Adding a Tauri command

Commands are type-safe via [tauri-specta](https://github.com/specta-rs/tauri-specta):

1. Add the command to a module in `src-tauri/src/commands/` with `#[tauri::command]` and `#[specta::specta]`
2. Register it in `collect_commands!` in `src-tauri/src/bindings.rs`
3. Regenerate bindings (`pnpm run rust:bindings`) and call it from React via `commands` in `@/lib/tauri-bindings`

See [docs/developer/tauri-commands.md](docs/developer/tauri-commands.md) for details.

## Documentation

- **[Developer Docs](docs/developer/)** — architecture, patterns, and detailed guides
- **[AGENTS.md](AGENTS.md)** — instructions for AI coding agents working in this repo

## License

Apache License 2.0 (see [LICENSE](LICENSE)), except the brand assets listed in [NOTICE](NOTICE). The Nandi name and logos are covered by [TRADEMARKS.md](TRADEMARKS.md).

The app works fully on its own. The optional relay that connects teammates is a separate product, [nandi-relay](https://github.com/vaultdbai/nandi-relay), under the Business Source License 1.1.

---

Built on [dannysmith/tauri-template](https://github.com/dannysmith/tauri-template) with [Tauri](https://tauri.app), [React](https://react.dev), and [shadcn/ui](https://ui.shadcn.com).
