# Contributing Guidelines

Thank you for your interest in contributing!

## Quick Start

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://rustup.rs/): the version in `rust-toolchain.toml` (rustup installs it)
- [pnpm](https://pnpm.io/) (never npm)
- Docker, to run agents
- Familiarity with React, TypeScript, and Rust

### Setup

Clone nandi-spec and nandi-relay next to this repo: the Rust build patches them to `../nandi-spec` and `../nandi-relay` (the relay is a test-only dev-dependency).

```bash
git clone https://github.com/vaultdbai/nandi.git
git clone https://github.com/vaultdbai/nandi-spec.git
git clone https://github.com/vaultdbai/nandi-relay.git
cd nandi
pnpm install
pnpm run dev
pnpm run check:all
```

## How to Contribute

### Issues

- **Bug Reports**: Use the bug report template
- **Feature Requests**: Use the feature request template
- **Security Issues**: See [SECURITY.md](SECURITY.md)

### Pull Requests

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/amazing-feature`
3. Make changes following the guidelines below
4. Ensure checks pass: `pnpm run check:all`
5. Commit using conventional commits
6. Push and open a Pull Request

## Code Guidelines

### TypeScript/React

- Use TypeScript for all new code
- Follow existing component patterns
- See `docs/developer/` for architecture patterns

### Rust

- Use `cargo fmt` and `cargo clippy`
- Use `Result<T, String>` for Tauri commands
- See `docs/developer/rust-architecture.md`

## Works Without the Relay

Nandi must stay fully usable with no relay configured or reachable. Only person-to-person features (asking or assigning someone else, the team inbox, reassignment, handover, presence) need the relay. See "Works Without the Relay" in `AGENTS.md`. `pnpm run test:offline` checks it.

## Quality Gates

All PRs must pass:

- TypeScript type checking
- ESLint and Prettier
- Rust formatting and clippy
- Tests

Run locally: `pnpm run check:all`, and `pnpm run test:offline` when relay or startup code changed.

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```bash
feat: add user authentication
fix(ui): resolve sidebar toggle issue
docs: update installation instructions
refactor(store): simplify state management
test: add preferences tests
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

## Code Review

- Keep PRs focused and reasonably sized
- Write clear PR descriptions
- Respond to feedback promptly
- Update documentation as needed

## Legal

Nandi is licensed under the Apache License 2.0 (see `LICENSE` and `NOTICE`). By contributing, you agree that your contributions are licensed under it too. The Nandi name, the bull and the logos are trademarks and are not covered: see `TRADEMARKS.md`.
