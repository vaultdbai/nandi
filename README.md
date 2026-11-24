<p align="center">
  <img src="src/assets/brand/mascot/companion-bison.webp" alt="The Nandi bison, lying down and breathing" width="240" />
</p>

<h1 align="center">Nandi</h1>

<p align="center"><strong>Give AI agents the job, not the keys.</strong><br />by <a href="https://vaultdb.ai">VaultDB.ai</a></p>

<p align="center">
  <a href="https://github.com/vaultdbai/nandi/releases/latest">Download</a> ·
  <a href="https://vaultdb.ai/">Nandi Business</a> ·
  <a href="https://vaultdb.ai/personal.html">Nandi Personal</a> ·
  <a href="SECURITY.md">Security</a>
</p>

Nandi is a free, open source desktop app that runs AI agents in a locked box on your own machine. It runs Claude Code, Codex and its own agents in a hardened container. Your model keys and sign-ins stay outside, the agent reaches only the sites you allow, tools that write files or run commands wait for your approval by default, and every run is on record.

> **Download** the latest release from [Releases](https://github.com/vaultdbai/nandi/releases/latest): Windows `-setup.exe` (no admin rights needed) or `.msi` for IT deployment, macOS `.dmg`, Linux `.AppImage`. Nandi needs Docker.

## Why

An AI agent is useful when it can act: run commands, edit files, call APIs. On a normal laptop that means it can reach everything the laptop can: cloud keys, tokens, customer data and the whole internet. This year an AI coding agent on a staging task found a broadly scoped token and deleted a company's production database and its backups in one call.

Nandi gives the agent the job without the keys.

## How Nandi Keeps Agents in the Box

| Area            | What Nandi does                                                                                                                                                                                                                                                                                                            |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **The box**     | Every agent runs in a Docker container: read-only root, all capabilities dropped, `no-new-privileges`, a non-root user, memory and process limits. Only the project folder and the folders you share with the agent are mounted                                                                                            |
| **Keys**        | Model keys and sign-ins never enter the container. Model calls go through Nandi's proxy on the host, which adds the real credential: Claude Team or Enterprise, AWS SSO through Bedrock, Azure Foundry, or a key from the OS keyring. An agent you grant AWS access gets temporary keys limited to the actions you granted |
| **The network** | The container has no route out except Nandi's egress proxy. Sites not on the agent's allow list are refused. The exceptions are the search engine, for web search, and the agent's model provider while a run lasts                                                                                                        |
| **Approvals**   | By default, tools that write files or run commands wait for your approval in the Inbox. You can relax that per agent, and the change is in its file                                                                                                                                                                        |
| **The record**  | Run history and every tool call are kept on your machine. The audit logs are hash-chained, so an edited line shows                                                                                                                                                                                                         |
| **Any agent**   | Claude Code and Codex run in the same container behind the same proxy. A test on every change checks each one's container for leaked keys and blocked sites ([`sandbox_suite.rs`](src-tauri/src/sandbox_suite.rs)). Google's Antigravity is next                                                                           |

**What Nandi doesn't protect against:** what you approve (an approved command does what it says, inside the container), a compromised host, and your model provider, since prompts go to the provider you choose. See [SECURITY.md](SECURITY.md).

## One App, Two Editions

- **Nandi Business:** for teams. An optional relay that your company hosts connects teammates' Nandis, so people can ask or assign work and share an inbox. The relay sees who asked whom, when, which job and its status. It never sees the request or the answer, which are sealed end to end. Free for up to 20 users.
- **Nandi Personal:** an AI assistant that asks first. It runs on your computer and keeps your notes. Its Gmail and Outlook connection is read-only: it can't send, move, delete or mark mail as read.

Both work without any server.

### What Leaves Your Machine

Nandi has no account and sends no telemetry. These are the only things it sends out:

- The model calls you chose, straight to your provider.
- With the free tokens: the text a task needs, through VaultDB.ai's trial server to DeepSeek. The server keeps none of the text.
- The sites you allowed, and mail and files read from Google or Microsoft if you connect them.
- Pushes to your own git remote, if you back up your agents.
- Your relay, if you join one. It sees who asked whom and when, never what was said.
- Update checks and ready-made agents from GitHub, container images from `ghcr.io`, and Claude Code from npm the first time you use it.

## Why It Runs on Your Machine

Model calls are the cost that grows. Running every agent, tool and coding CLI in a hosted service on top of them makes it grow faster: hosted compute is billed per minute, and a team of agents runs all day. Most people already have a desktop or laptop powerful enough to run agents in containers, and it sits idle much of the time. Nandi uses that machine:

- **Cost:** you pay only for model calls. Agents, tools, containers and storage run on hardware you already own. Local models through Ollama are an option for work that doesn't need a hosted model.
- **Your setup:** agents use your non-secret settings (Claude settings, `.npmrc` and `pip.conf` without tokens, `~/.aws/config`), mounted read-only, so there's nothing to rebuild in someone else's cloud.
- **Your data:** files, history and definitions stay on your disk (`~/.nandi`).

## Stack

| Layer    | Technologies                                    |
| -------- | ----------------------------------------------- |
| Frontend | React 19, TypeScript, Vite                      |
| UI       | shadcn/ui, Tailwind CSS v4, Lucide React        |
| State    | Zustand v5, TanStack Query v5                   |
| Backend  | Tauri v2, Rust, bollard (Docker API)            |
| Testing  | Vitest, Testing Library, cargo test             |
| Quality  | ESLint, Prettier, ast-grep, knip, jscpd, clippy |

## Build From Source

### Prerequisites

- [Node.js](https://nodejs.org) 20+
- [pnpm](https://pnpm.io) 9+ (`corepack enable`)
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
├── rpc/                      # Each command declared once, for desktop and browser
├── desktop/                  # Desktop-only Tauri commands (windows, file picker, ...)
└── bindings.rs               # tauri-specta command registration
locales/                      # Translations (en, fr, ar)
```

### Adding a Tauri command

Commands are type-safe via [tauri-specta](https://github.com/specta-rs/tauri-specta):

1. Declare it in one line with `rpc::commands!` in `src-tauri/src/rpc/<module>.rs`: it serves Tauri IPC and `POST /api/rpc/{name}`. Only a command that needs a window or the `AppHandle` is hand-written, in `src-tauri/src/desktop/`
2. Register it in `collect_commands!` in `src-tauri/src/bindings.rs`
3. Regenerate bindings (`pnpm run rust:bindings`) and call it from React via `client` in `@/lib/client`

See [docs/developer/tauri-commands.md](docs/developer/tauri-commands.md) for details.

## Documentation

- **[Developer Docs](docs/developer/)**: architecture, patterns, and detailed guides
- **[AGENTS.md](AGENTS.md)**: instructions for AI coding agents working in this repo

## Contributing

Bug reports and pull requests are welcome.

- **Bugs and ideas:** open an [issue](https://github.com/vaultdbai/nandi/issues). Include your Nandi version and the error.
- **Security holes:** don't open an issue. Follow [SECURITY.md](SECURITY.md).
- **Pull requests:** read [AGENTS.md](AGENTS.md) for the patterns, and run `pnpm run check:all` before you push.
- **The app works without the relay.** Every feature except person-to-person work must work with no relay configured.

## License

Apache License 2.0 (see [LICENSE](LICENSE)), except the brand assets listed in [NOTICE](NOTICE). The Nandi name and logos are covered by [TRADEMARKS.md](TRADEMARKS.md).

The app works fully on its own. The optional relay that connects teammates is a separate product, [nandi-relay](https://github.com/vaultdbai/nandi-relay), under the Business Source License 1.1.

---

Built on [dannysmith/tauri-template](https://github.com/dannysmith/tauri-template) with [Tauri](https://tauri.app), [React](https://react.dev), and [shadcn/ui](https://ui.shadcn.com).
