# Nandi Quickstart

From nothing to an agent that runs a command for you, in about fifteen minutes. The full [User Guide](./userguide.md) explains each step.

## 1. Install the prerequisites

- **Docker** ([Docker Desktop](https://docs.docker.com/get-docker/) on Windows and macOS, Docker Engine on Linux), running.
- **Nothing to build:** Nandi downloads its sandbox images from `ghcr.io/vaultdbai` when an agent first needs them (or click **Download** in Settings › Models & runtimes). Offline, **Build here** in the same place builds them on this computer.

- **Nandi** itself, from the [downloads page](https://vaultdb.ai/downloads/nandi) (placeholder link: Windows `-setup.exe`, no admin rights needed, or `.msi` for IT; `.dmg`; `.AppImage`), or from source with `pnpm install && pnpm run tauri:dev`.

## 2. Set up Nandi

1. Launch Nandi and type your name.
2. **Connect a model**: paste an Anthropic, OpenAI or DeepSeek key and click **Test** (or **Use …** if Nandi found one on this computer). It goes into your OS keychain. You can change it later in **Settings › Models**.
3. **Network**: **Not now** is fine for this quickstart.
4. Pick a starter pack or two (Research and Engineering are good first choices), or **Skip**.

## 3. Create an agent

Save this as `~/.nandi/repo/.nandi/agents/shell-helper/skill.md`:

```markdown
---
name: Shell Helper
description: Runs commands in its sandbox and explains the output
model: deepseek:deepseek-flash
allowed-tools: [shell, read_file, write_file, list_dir]
requires_approval: [shell, write_file]
---

You are a careful assistant with a Linux sandbox. Before running a command,
say what it does. After running it, summarize the output in two sentences.
```

It appears in the sidebar within a second.

## 4. Give it a task

Open **Shell Helper** and send:

```text
Run "uname -a && df -h /" and tell me what kind of machine this is.
```

The container starts, the model plans, and a `shell` call waits for you. Click **Approve**, then read the answer. Open **Home › All activity** to see the run's timeline, tokens, and cost.

## 5. Next steps

- **Share it:** account menu → **Sync now** commits and pushes the agent to your repo.
- **Automate it:** **Triggers → New Trigger** with a schedule like `0 9 * * 1-5`.
- **Chain agents:** a workflow YAML in `~/.nandi/repo/.nandi/workflows/`.
- **Something wrong?** See [Troubleshooting](./userguide.md#12-troubleshooting). The usual culprit is Docker not running.
