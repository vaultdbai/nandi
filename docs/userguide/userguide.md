# Nandi User Guide

This guide is for people who are comfortable with a terminal and git but new to AI agents. It is opinionated: where there are several ways to do something, it tells you the one we'd pick. For the one-page version, see the [Quickstart](./quickstart.md).

**Contents**

1. [What Nandi is](#1-what-nandi-is)
2. [Installation](#2-installation)
3. [First launch](#3-first-launch)
4. [Your first agent](#4-your-first-agent)
5. [Managing your team of agents](#5-managing-your-team-of-agents)
6. [Workflows](#6-workflows)
7. [Triggers](#7-triggers)
8. [Library](#8-library)
9. [Teams and collaboration](#9-teams-and-collaboration)
10. [Relay and remote delegation](#10-relay-and-remote-delegation)
11. [Local models](#11-local-models)
12. [Troubleshooting](#12-troubleshooting)
13. [Security model](#13-security-model)

---

## 1. What Nandi is

Nandi is a desktop app for running a small team of AI agents on your own computer. You are the manager. Each agent is a model (Claude, GPT, DeepSeek, or Ollama: your default model in Preferences → Advanced, or whichever provider you have added a key for) with a job description, a set of tools, and its own sandboxed Docker container to work in. You give agents tasks, approve the risky things they want to do, and review what they did afterwards.

Everything that defines your agents is a plain file in a git repository you own. An agent is a folder with a `skill.md` file: YAML frontmatter for its settings, followed by its instructions in Markdown. Workflows and triggers are YAML files next to them. Nandi keeps that repository at `~/.nandi/repo` and syncs it with GitHub, so your agents are versioned, reviewable, and portable between machines. Run history, approvals, and logs stay local in `~/.nandi/local`.

Nandi is local-first. The app, your agents' containers, and their files run on your machine. The only things that leave it are the model calls your agents make (through a proxy that holds your API keys), the network requests you explicitly allow, and git syncs with GitHub. Sharing with teammates happens through GitHub repositories and, optionally, a relay on your local network.

---

## 2. Installation

You need three things: the Nandi app, Docker, and an API key for a model provider.

### Requires Docker

Agents run their tools inside a container made from Nandi's sandbox images. Install Docker first:

- **Windows and macOS:** [Docker Desktop](https://docs.docker.com/get-docker/). Start it and leave it running.
- **Linux:** Docker Engine from your distribution or docker.com. Add yourself to the `docker` group so Nandi can reach the daemon without `sudo`.

Without Docker, Nandi still opens and works as a chat assistant: agents can talk, not act (no files, shell, web, connectors, coding agents or headless browser).

**Get your computer ready.** Nandi checks this for you, read-only, during setup (the **Computer** step, which you can skip) and any time in **Settings › This computer**. Each row says **Ready**, **Missing**, **Not running**, **Needs an update**, **Blocked** or **Couldn't tell**, with what Nandi saw:

| Row              | Windows                                                           | macOS                                                            | Linux                                                                                     |
| ---------------- | ----------------------------------------------------------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| Docker installed | Docker Desktop                                                    | Docker Desktop, the build for your chip (Apple silicon or Intel) | Docker Engine or Docker Desktop (Podman and rootless Docker are noted, not supported yet) |
| Docker running   | yes                                                               | yes                                                              | yes (and whether your account may use the Docker socket)                                  |
| WSL 2            | installed and 2.1.5 or later                                      | —                                                                | —                                                                                         |
| Virtualization   | turned on in the firmware                                         | Apple silicon, or Intel VT-x                                     | —                                                                                         |
| Disk space       | about 15 GB free where Docker keeps images                        | same                                                             | same                                                                                      |
| Nandi's images   | downloaded, or `ghcr.io` reachable (network, proxy, certificates) | same                                                             | same                                                                                      |

Each row that isn't ready says in one sentence why it matters, then numbered steps for your computer, with commands you can copy and links to Docker's and Microsoft's official pages. **Start Docker** opens Docker Desktop (no admin rights). On Linux, Nandi shows the commands for your distribution (Debian/Ubuntu, Fedora/RHEL, Arch) to run in a terminal; it never runs `sudo` itself. For "Docker Desktop is unable to start", the steps go in order: quit Docker Desktop, `wsl --update`, `wsl --shutdown`, start it again, check virtualization, and as a last resort Docker's own reset. The page checks again every few seconds while it's open; **Check again** re-runs everything now. Until Docker is ready, Home shows a calm "Agents can talk, but need Docker to act. Set up", and an agent page shows "Agents need Docker to act. Set up" instead of a warning.

You don't need to build anything. Nandi downloads the two images it needs from GitHub's container registry, the version that matches your Nandi:

| Image                             | What it is for                                           |
| --------------------------------- | -------------------------------------------------------- |
| `ghcr.io/vaultdbai/nandi-runtime` | The tool sandbox of Nandi's own agents (commands, files) |
| `ghcr.io/vaultdbai/nandi-harness` | Agents that run in Codex, Gemini CLI or Claude Code      |

The first agent that needs one starts the download by itself and waits for it. A small notice shows the size and how far it is, whichever page you're on. You can also get them ahead of time in **Settings › Models & runtimes** (**Download now**). If a download fails (offline, or `ghcr.io` blocked), the notice and the page say why and offer **Try again**.

**Claude Code is fetched on first use.** Nandi doesn't ship Claude Code inside its image. The first time an agent runs in Claude Code, Nandi gets it from npm on this computer (one time, about 237 MB) and keeps it in a Docker volume for later runs. A notice says "Getting Claude Code (one time, about 237 MB)…". You can also click **Get it now** under **Settings › Models & runtimes › Coding agents**. You accept Anthropic's terms when you sign in to Claude Code, as you do anyway. When a Nandi update needs a newer Claude Code, it's fetched again the same way. Codex and Gemini CLI are in the image, so they need no extra step.

If you want, **Settings › Models & runtimes › Advanced** can also build the images on this computer instead of downloading them. Nothing needs that.

Without the tool sandbox, agents still answer, but in text only: Nandi shows a "Tool execution unavailable" banner and the agent can't run commands or touch files.

### The app

Download the installer for your platform from the [Nandi downloads page](https://vaultdb.ai/downloads/nandi) (placeholder link):

| Platform | Installer                      | Notes                                                                                                                 |
| -------- | ------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| Windows  | `-setup.exe`                   | Windows 10 or 11. Installs for you only: no admin rights needed. WebView2 ships with Windows and installs if missing. |
| macOS    | `.dmg`                         | Drag Nandi to Applications.                                                                                           |
| Linux    | `.AppImage`, `.deb`, or `.rpm` | The AppImage needs `chmod +x`. WebKitGTK is a dependency of the packages.                                             |

Installed builds update themselves: when a new version is ready, an **Update** button appears in the title bar.

#### For IT: deploy to many machines

`VaultDB-Nandi-windows-x64.msi` installs Nandi for every user of a machine (per-machine, under Program Files), so it needs admin rights. Push it with Intune, Group Policy or SCCM, or install it silently:

```powershell
msiexec /i VaultDB-Nandi-windows-x64.msi /qn
```

Updating a per-machine install needs admin rights too. Update it with your own tooling when a new version is out. Users without admin rights still see **Update** in the title bar; clicking it asks Windows for permission, and if they can't give it nothing changes and Nandi keeps working (**Retry update** stays until IT updates it). Individuals should use the `-setup.exe`, which updates itself without admin rights.

**From source** (any platform), you need Node.js 20+, pnpm, Rust, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/):

```bash
pnpm install
pnpm run tauri:dev
```

### A model API key

Agents use DeepSeek unless their `skill.md` says otherwise. Open **Settings › Models** and paste a DeepSeek, Anthropic, or OpenAI key. Keys go into your OS keychain; Nandi never shows them again and never passes them to agent containers.

---

## 3. First launch

![screenshot: Welcome to Nandi](placeholder)

The first launch is five short steps: **Name → Model → Email → Network → Agents**. Each step saves as you leave it, so if you quit midway Nandi opens where you stopped (**Skip the rest of setup** jumps straight in).

### 1. Name

Type your name. Nandi derives your **handle** from it (`Ada Lovelace` → `@ada-lovelace`); others use it to find you. **Customize** lets you pick a different one: lowercase letters, numbers and hyphens.

### 2. Connect a model

<!-- studio-guide: runtimes-and-sign-in -->

Agents think with a model from a provider you choose. Keys go into your OS keychain, never into a file.

- **Found a key for Anthropic on this computer**: another Nandi on this computer already saved one. **Use Anthropic** takes it in one click.
- **Anthropic, OpenAI, DeepSeek**: pick one, paste a key, and click **Test**. Nandi checks the key with the provider (no tokens are spent) and keeps it only if it works. If you're offline, **Save without testing** keeps it anyway.
- **Ollama**: if it's running on this computer, pick one of its models. No key needed.
- **Amazon Bedrock** works through the Claude Code runtime; set it up later under **Settings › Models & runtimes** and **Settings › Cloud credentials**.

**Free tokens:** Nandi comes with 100,000 free tokens on DeepSeek Flash, so you can start without a key. They switch off once you set up your own model. While some are left, you can turn them back on in **Settings › Models › Free tokens**.

**Skip for now** asks once more, because without a model agents can't run. You can add one any time in **Settings › Models**.

### 3. Connect your email (optional)

Pick the mailboxes Chief of Staff should read, **Outlook**, **Gmail** or both, and connect each in turn: Nandi opens your browser to sign in. The step says what Nandi reads and what it never does (see [Connect your email](#connect-your-email)). Choose how often Chief of Staff checks: 5, 15 or 30 minutes (the default), 1 or 2 hours. A mailbox your Nandi can't connect says "Not available in this build".

**Skip for now** goes on without email; Home then shows **Connect your email** until you connect one. When setup ends, Chief of Staff reads the last 14 days straight away, and the name and address of the mailbox go into its **My role** (edit them on the Chief of Staff's page).

### 4. Work with other people? (optional)

**Not now** keeps everything on this computer, and nothing connects. **Join the Nandi network** (when your Nandi comes with one) or **Join my team's network** (the relay's address and its join code, or **Paste invite**) joins a relay when you click **Join** or **Continue**; nothing joins, and no seat is taken, before that. If joining doesn't work, Nandi says why and **Continue anyway** goes on without a network. **Let others send requests to my Nandi** is off until you tick it; even then, a request only gets through once an agent accepts requests and you trust the person asking. You can do all of this later in **Network**.

### 5. Starter packs

Pick one to three packs of ready-made agents and workflows. Today's packs:

| Pack        | Agents                                                    | Workflow            |
| ----------- | --------------------------------------------------------- | ------------------- |
| Research    | arxiv-researcher, paper-summarizer, citation-formatter    | `literature-review` |
| Engineering | code-reviewer, test-writer, docs-generator, pr-summarizer | `pr-review`         |
| Content     | blog-draft-writer, newsletter-formatter, seo-analyzer     | `weekly-content`    |
| Analytics   | data-fetcher, chart-generator, report-writer              | `daily-metrics`     |
| Personal    | email-triager, calendar-summarizer, meeting-notes         | `daily-briefing`    |

They install in sequence with progress. A pack that fails can be retried, or left out with **Continue without them**. You then land on the Dashboard, which names the agents you got and suggests something to try. **Skip** installs nothing; the Dashboard's **Get started** cards (install a pack, create an agent by describing it, join a network) are there whenever you want them.

Once something has run and nothing is backed up yet, the Dashboard offers **Set up backup** (a git remote, in **Settings › Backup**).

### Where settings live

**Settings** (sidebar, or Ctrl+,) is a full page with one section per job, and each setting lives in exactly one place:

| Section               | What's there                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Models & runtimes** | What runs your agents, on one page. **Nandi (built in)**: the free tokens (a switch), the default model picked from a list (models whose provider has no key say so; **Custom…** takes any `provider:model`), a line saying which model calls really use and why, and the **Tool sandbox** image (Nandi downloads it when an agent first needs it; **Download now** gets it ahead of time, with its size and progress). **Coding agents**: the **Coding agents' sandbox** image the same way, **Claude Code** (fetched on first use; **Get it now**), then Claude Code, Codex and Gemini CLI with their status and Bedrock models. **Provider keys** (Save runs a test; **Remove** asks first) and Ollama. Team model settings, sessions and the optional local build of the images under **Advanced** |
| **Account**           | Your display name and handle. Changing the handle while backup is set up asks first, because your backup branch changes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| **Auto-approve**      | Rules that let some items through without a click: mail from a domain (`chase.com`) or an address, or requests from a teammate. See [Auto-approve rules](#auto-approve-rules)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| **Cloud credentials** | AWS: the profile or SSO sign-in for Bedrock, and the access key agents' AWS calls use. Only the account's last 4 digits are shown                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| **Internet access**   | Each agent's allowed sites (change them on the agent), recent requests, and the proxy under **Advanced**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| **Backup**            | The git remote: **Back up now** (asks first) and **Restore from remote**, plus this computer's copy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| **Sources**           | Where the Library gets agents, packs and capabilities, in one list with **Sync** per row and **Sync all**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| **Logs**              | Model calls, internet, triggers, pilots and connectors                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| **Appearance**        | Light or dark, theme, language, the companion, and the bull's name                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

The relay is not here: it's in **Network › Connection**.

---

## 4. Your first agent

There are two good ways to get an agent of your own. The fastest is to fork one you installed; the most instructive is to write one by hand. Do the second once, so you know what an agent really is.

### Write one by hand

<!-- studio-guide: build-an-agent -->

Create a folder in your agents repo and a `skill.md` inside it. The folder name is the agent's ID: lowercase letters, digits, and hyphens.

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

Save it as `~/.nandi/repo/.nandi/agents/shell-helper/skill.md`. Nandi watches the repo, so the agent appears on the **Agents** page within a second. No restart, no import. If the file is invalid, Nandi keeps the last good version and tells you what's wrong.

The frontmatter keys you'll use most:

| Key                   | What it does                                                                   |
| --------------------- | ------------------------------------------------------------------------------ |
| `name`, `description` | What you and other agents see                                                  |
| `model`               | `<provider>:<model>`: `deepseek:`, `claude:`, `gpt:`, or `ollama:`             |
| `allowed-tools`       | Which container tools it may use. Leave it out for all of them                 |
| `requires_approval`   | Tools that need your OK before every call                                      |
| `network`             | Hosts it may reach (see [Security model](#13-security-model)). None by default |

Prefer putting long-lived standing instructions in an `instructions.md` next to `skill.md`: it's appended to the system prompt and keeps the frontmatter file short.

**Playbooks.** An agent can have step-by-step guides for the kinds of work it does, in a `skills/` folder next to `skill.md`: one Markdown file per playbook, `skills/<name>.md`.

```markdown
---
name: pay-a-bill
description: Pay a bill on the biller's website, with you making the payment
when: A bill is due, or you ask it to pay one
---

1. Plan the run first ...
```

- `name` is lowercase words joined by hyphens and must match the file name. `description` and `when` are one line each. The steps stay at 900 words or fewer. A playbook sets no `model`.
- Every run lists only each playbook's name, description and when to use it. The agent reads a playbook's steps (`skill_get`) when a task calls for it, so an agent can know many playbooks and still start quickly.
- Open an agent's **Playbooks** tab to read them, change one, add a new one or delete one. Saving checks the same rules. An installed agent's playbooks are read-only, like its `skill.md`.
- **"Remember how I do this."** Tell an agent you made (Chief of Staff too) how you want something done, for example "remember: I pay bills from Chase, never the card". It drafts a playbook and asks you first: the card shows its name, what it covers, when it's used and the steps (**Show the steps**). A change to a playbook it already has says "replaces the current one" and shows what changes (**Show the changes**). **Approve** saves it to `skills/`; **Reject** saves nothing. Installed agents can't do this.

### Or fork an installed agent

Installed agents are read-only. Open one, click **Fork** in the banner, and give it a new ID and name. The copy is yours: fully editable, and no longer updated from the marketplace. Forking is how you customize anything you installed.

### Send the first message

![screenshot: Agent view with a tool call card](placeholder)

Open the agent and type a task:

```text
Run "uname -a && df -h /" and tell me what kind of machine this is.
```

Then watch it work:

1. **Starting agent runtime…** The first message starts the agent's container (`nandi-agent-<tag>-shell-helper` in `docker ps`, where `<tag>` is eight letters and digits that tell your Nandi's containers apart from another Nandi's on the same computer). It stays up for later messages.
2. **Thinking…** Tokens stream in as the model plans.
3. A **tool call card** appears for `shell`. Because `shell` is in `requires_approval`, it waits for your approval: the card says what it will do in plain words ("Run a command: uname -a"), how long it has left, and the raw arguments under **Details**, with **Approve** and **Reject**.
4. After you approve, the card shows the result (`stdout`, `exit_code`), the model reads it, and it writes the final answer.

Each agent works on **one request at a time**. Send another while it's busy and it's queued: the strip above the chat says "+1 queued", and **Show queue** lets you drag requests to reorder or remove them. **Stop** cancels the current turn.

### What the agent sees

Inside its container, the agent's folder is mounted read-only at `/agent/skill` and its working folder (`~/.nandi/local/work/<agent-id>` on your machine) read-write at `/agent/workdir`. File paths in its tools are relative to that working folder. Nothing else on your disk is visible to it.

---

## 5. Managing your team of agents

### Finding your way

The sidebar has seven pages. Three open one level down: **Ask** (**+ New chat**, which opens an empty chat that's saved once you send its first message, your five latest chats and **All chats**), **Agents** (each agent, with a dot for what it's doing, busiest first) and **Automations** (**Workflows** and **Triggers**). Home, Library, Network and Settings are single items; their pages have their own tabs. Click a name to open it (and its branch), or the arrow beside it to open or close the branch only. Branches stay as you left them, and the one above the page you're on opens by itself.

| Item            | What's there                                                                                                                                                                                       |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Home**        | **Inbox** (what waits on you; its count is on the sidebar badge), **Running**, **Recent**, and **All activity →** (every run and delegation). Type into **Ask Nandi…** to start a chat in **Ask**. |
| **Ask**         | A chat with your Nandi: ask questions, run things, or build agents, workflows and policies. **New ▾** starts a chat or something to build.                                                         |
| **Agents**      | Every agent as a row with its runtime, last 7 days and sharing. Click one for its page (Overview, Chat, Runs, Settings); **New agent** makes one.                                                  |
| **Automations** | **Workflows** and **Triggers**, each with its **New** button.                                                                                                                                      |
| **Library**     | **Installed**, **Browse** and **Shared with me**: agents, workflows, tools (built-ins, connectors, pilots) and packs, each with one card.                                                          |
| **Network**     | **People**, **Teams**, **Sharing** (Gateway, Shared by me, Trust graph, Capabilities) and **Connection** (the relay). A dot shows whether you're connected.                                        |
| **Settings**    | **Models & runtimes**, **Account**, **Cloud credentials**, **Internet access**, **Sources** and **Logs**.                                                                                          |

Up and Down move through the sidebar, Right opens a branch (or steps into it), Left closes it (or goes back up), and Enter opens the page. **Ctrl+K** (⌘K) opens the command palette; its first row, **Ask Nandi: …**, sends what you typed to **Ask**.

### Home

Home answers "does anything need me, and is everything else moving?". From the top:

- **Ask Nandi…** sends what you type to **Ask**.
- One line about your network: "Connected · 4 people online", or "Not connected" with **Join a network**.
- Up to three numbers (**Runs today**, **Failed in 24 h**, **Waiting on you**), once there's something to count. Click one for that slice of All activity.
- The **Inbox**: everything waiting on you, each with its fix right there (see Approvals below).
- **Running**: what's going now, with "step 2 of 4" and how long it's taken. Something with no progress for 10 minutes says so.
- **Recent**: what finished in the last day. Rate a delegation's result once (★); a failure has **Retry** (a workflow asks first, since it runs every step again).

When nothing needs you and nothing runs, Home says so in one line. On a fresh install it shows **Get started** instead. Everything updates live; leave it open on a second monitor if you run agents on triggers.

**All activity** (from Recent) lists every run and delegation, both ways, in one table. Filter by search, date, status, source (you, a trigger, a schedule, a workflow, another Nandi), direction, agent and member; the filters stay put when you open a row and come back. **Export CSV** saves exactly what the filters show.

### Approvals

<!-- studio-guide: approvals-and-policies -->

An approval is a tool call that is waiting for your approval. The agent is suspended, not spinning: it uses no tokens, and the approval survives an app restart.

Every approval waiting on you is in the **Inbox** on **Home** (the number on **Home** in the sidebar), together with workflow steps waiting for your approval, questions another Nandi asked you, manual workflow steps, and a missing model. Each has its fix right there. Failures from the last day are listed after them with **Retry** and **Dismiss**; they aren't counted in the number. You can decide the same approval in the chat, in the run's **Approvals** tab, on its Request page, or in the Inbox: deciding it in one place clears it everywhere.

- **Approve** runs the call; the agent continues.
- **Reject** asks you to confirm, then skips it; the model is told, and should adjust its plan rather than retry.
- An approval nobody decides **expires after an hour**, the same as a reject: the run goes on without that call (or stops, if it can't). The card counts down ("Expires in 12 min").

Approvals work the same on every runtime. On Claude Code, Codex or Gemini CLI, the CLI waits while its tool call is in your Inbox: approve and it runs, reject (or leave it an hour) and the CLI is told no. If Nandi quits while a call waits, that run is cancelled.

Our advice: put `shell` and `write_file` in `requires_approval` for any new agent, run it a few times, then relax it once you trust its behaviour. Approvals for agents you didn't write (marketplace, teammates) are the cheapest insurance you have.

**Requests from other Nandis that need your approval.** If you set one of your gateway's capabilities to **Needs my approval**, a request for it waits instead of being turned away. It shows in the Inbox as "Sam asked for finance.risk", with what they asked, when it's due, **Approve** and **Decline**. Approve and the request goes on. Decline asks you first, then Sam sees "You declined this request" and can send it elsewhere. Nobody deciding by its due date counts as no. While it waits, Sam sees "Waiting for you to approve this request" on their Request page. A workflow step waiting for approval can be approved or rejected from the Inbox too.

**When your Nandi is busy**, a request from another Nandi waits in line instead of being refused. Its Request page shows "Place in line: 2", and the sender sees the same. It waits until the request is due, then stops as timed out.

**When an answer comes back,** you're told: the Inbox shows "Lena answered" with what she said (or "marked it done", "declined it", or "no answer by the due date"), and the companion sends a notification while it's hidden. **Dismiss** takes it off the list. Turn the notifications off in Settings › Appearance › **Notify me when requests come back**. Answers that arrive while you're still in the chat show there instead.

**Requests last until they're due.** A question or request you send to someone's Nandi has a due date: the one you give ("due Friday"), else 2 days for a question and 1 day for anything else (7 days for assigned work). If they don't answer within a few minutes, Chief of Staff tells you it's waiting and you carry on. If they're away, it reaches them when they're back. The answer comes back on its own, and "What am I waiting on?" shows it. Past its due date, the request ends and says so.

Two kinds of approval are not per call:

- **Marketplace consent.** An installed agent can't run until you allow each capability it requests (tools, network hosts, AWS, pilots). **Install and allow** in the Library does it at install; an agent installed another way asks with **Review and allow** before its first run.
- **Pilot consent.** Pilots (host-side OS control, see the **Pilots** page) need a per-agent, per-capability grant. Every pilot tool also asks per call unless it's marked read-only.

### Reviewing runs

Every turn is a **run**. Open **Home › All activity** (or an agent's **Runs** tab, which shows what each run was **Part of**: Chat, a workflow, a trigger or a request) for the history with tokens and cost. Click a run for its detail view:

| Tab       | Shows                                                            |
| --------- | ---------------------------------------------------------------- |
| Overview  | Status, source (you, a workflow, a trigger, another agent), cost |
| Timeline  | Every step, tool call, and result in order                       |
| Outputs   | The agent's final reply                                          |
| Files     | Files it touched                                                 |
| Logs      | Raw events                                                       |
| Analysis  | Ask questions about the run ("why did step 3 fail?")             |
| Approvals | What it asked for and what you decided                           |

**Settings › Logs** collects model calls (model, tokens, cost; never prompt text), internet requests, trigger fires, pilot actions, and connector calls.

### Editing an agent

Each agent's page has four tabs:

- **Overview**: a health card (runs, failures and cost over the last 7 days, one square per day) with the fix for any problem ("Pick a model", "Add a key"), what waits for you, recent work, and which workflows use it.
- **Chat**: talk to this agent; it has its own message box.
- **Runs**: every run with its status, filterable, and what it was part of.
- **Settings**: **General** (name, description), **Model** (runtime; for Nandi, a provider and model by name, where providers without a key are greyed out; for a CLI runtime, the CLI's model; it always runs in a container), **Permissions** and connectors, **Sharing** ("Accepts requests from other Nandis" asks before turning on), **Advanced** (container image, memory, CPUs, memory notes) and **Delete**, which asks you to type the agent's name. **Save** writes the agent's files.

To change instructions or tools, click **Edit in Ask** in the page header. **New agent** on the Agents page starts from a template (Blank, Researcher, Writer, Helper) with your default model, or hands the description to **Ask**. Start, stop and restart a container agent from the page's **⋯** menu.

Editing the files directly is equally valid. Either way, the change is staged in git; **Sync now** in the account menu commits, pulls, and pushes everything.

**Your rules.**

- Every agent's Settings has **Your rules**: its `instructions.md`, standing rules added to every run ("always cite the ticket", "never touch prod").
- For an agent installed from Library, they're saved as your own copy beside the install. The agent stays installed and updatable, and updates keep your rules. **Reset to default** goes back to the rules it shipped with.
- You can also ask in Ask: "make PR Reviewer flag every TODO".

### Turn limits, memory, and self-checks

Recent releases add a few behaviours you control from `skill.md`:

```yaml
max_turns: 50 # stop the run after this many steps (default 50)
max_cost_usd: 5.0 # stop once the run has cost this much (default $5)
middlewares: [verify_on_stop, compaction, plan_first, finish_check] # the default; [] turns them off
memory_disabled: false # true: no memory.md for this agent
```

- **Limits.** A run that hits `max_turns` or `max_cost_usd` is stopped, and the reason shows in the run. Lower `max_cost_usd` on agents that run unattended.
- **Memory.** After a run, the agent may save up to three durable facts ("the user's repo uses pnpm") to `memory.md` in its folder, and memory is loaded into its prompt on later runs. It's a file in your repo like everything else; review and edit it in the agent's **Memory** tab. Turn it off with `memory_disabled: true` or the tab's switch.
- **Verify on stop.** If an agent wrote files and is about to finish without checking them (running tests, a type check, re-reading the file), it gets one nudge to verify first. It's a heuristic; firings are logged to `~/.nandi/local/logs/verify.log`.
- **Compaction.** When a long run nears the model's context window, it ends as **compacted** and a continuation run picks up from a summary plus the most recent steps. The run detail links the two ("continued from").
- **Many tools.** An agent with more than 15 tools (say, several connectors) starts with its core tools plus `tool_search`, and loads the rest by searching for them as needed. You'll see `tool_search` calls in the timeline; they never need approval.

### Looking at one run

Open a run from an agent's **Runs** tab, a Request's step, or Home. The title says which agent, what it was asked, and when.

- **While it runs**, the page opens on **Timeline** and updates as the agent works; the time ticks. **Stop** ends it. If the run already wrote files, sent something, or is waiting for your approval, Nandi asks first.
- **Once it's done**, the page opens on **Overview**. **Re-run** sends the same input to the same agent again and keeps you on the page; it asks first when the run wrote, sent, ran a command, or asked another agent.
- **Timeline** shows each step to the second and how long it took.
- **Logs** has one readable line per event. Filter by Model, Tool, Approval, Error or Other, or search; click a line to see its raw data. **Copy log** copies everything.
- **Files** lists what the run read and wrote. **Open** and **Show in folder** work in the desktop app; **Copy path** works everywhere.
- **Approvals** lists what the run asked you. For a run on Claude Code, Codex or Gemini CLI that asked nothing, it lists what the agent's allowed tools let the runtime do or refused.

### Ask and the companion

<!-- studio-guide: companion-and-appearance -->

**Ask** is a chat with the workspace itself: ask it to draft a `skill.md`, design a workflow, or explain an error. It looks things up in your Nandi (runs, Requests, agents, workflows) and reads its built-in guides on how to do things. It shows you the file, then saves it once you approve the save card: new agents, workflows, triggers, connectors (tools from outside services and shared folders), capabilities and policies all land in your repo that way, and open in the editor on the right of the page for changes. Ask it to "run the weekly report" and it shows you a plan; nothing starts until you approve it. The **companion** is the bison in a corner of your screen, above your other apps, lying down and breathing, with a count when something needs you. It moves by default; Settings › Appearance › **Bison moves** sets it to Never, or to follow your computer's reduce-motion setting (on Windows, Animation effects). Drag it to any corner; it snaps there and stays after a restart. Click it, or press Cmd/Ctrl+Shift+Space, to see what needs you, what's running and what finished recently; you can approve or answer right there. Ask it about your work in plain words ("what did my agents do today?"), or start with "@Agent name" to talk to one agent. Requests it mentions appear as live cards. Clicking something that's running or finished keeps you in the companion: it's pinned at the top as a live card, where **Ask about this** points the chat at it; its **›** opens the full page in Nandi. On a Request's page, **Watch in companion** does the same. Clicking elsewhere shrinks it back to the bison; to have no bison at all, turn off **Nandi on the desktop** in Settings › Appearance, and the shortcut shows and hides the card instead. Closing the main window leaves the companion running: **Open in Nandi** brings the window back, and **Quit Nandi** in its ⋯ menu quits. When the card is closed it notifies you about new approvals and questions; turn this off in its ⋯ menu or in Settings › Appearance. To change an agent or workflow, it hands you off with **Open in Ask**. **Cmd/Ctrl+K** opens the command palette for everything else.

### What runs on its own, and what doesn't

<!-- studio-guide: how-nandi-works -->

You can ask Ask any of these questions yourself, for example "Does Chief of Staff keep watching my work?" or "When does it update my notes?". It answers from the same built-in guide this section comes from.

- **Nothing runs unless something starts it.** An agent runs when you message it, when a Request or workflow step reaches it, when a trigger fires, or when someone on your network asks it something. Between runs no agent watches anything and no model is called. Triggers fire only while Nandi is running. Requests and assigned work pick up where they were after a restart.
- **How Chief of Staff keeps track.** It keeps no list of its own. Ask "What am I waiting on?" and it reads the app's records of what you asked others and what they asked you, with status, due dates and what's overdue. When a teammate clicks **Done** or **Decline**, Nandi records it straight away, even if Chief of Staff isn't running. You see it the next time you ask. The arrival doesn't start Chief of Staff, so it won't tell you by itself.
- **When it updates your notes.** Only during a conversation with you, and only when it judges something worth keeping: the first get-to-know-you questions, something you mention (a deadline, decision, owner change or contact), or a request result that comes up in the chat. Work teammates assign you goes in too: early in each chat it checks what's on your list and notes what's new (who asked, what, when it's due) and what you marked done or declined. It says where it noted it. It never changes your notes while answering a teammate's question, and Ask never writes notes. If you don't talk to it, the notes stay as they are.
- **Want it to check in on its own?** Add a schedule trigger that messages Chief of Staff, for example at 9:00 on weekdays: "Check my deadlines and open requests and tell me what needs my attention today." Its answer is in the trigger's **Firings**. Asking or assigning anyone still waits for your approval.
- **Updates** happen only when you choose Library › Installed › **Check for updates**.

---

## 6. Workflows

<!-- studio-guide: agent-or-workflow -->

A workflow chains agents: each step sends a message to an agent and waits for its final reply before the next step starts. Use one when a task has a clear hand-off ("research, then summarize, then post"). If you just want one agent to do several things, put that in its instructions instead.

### Build one

<!-- studio-guide: write-a-workflow -->

Open **Automations → Workflows** and click **New workflow**:

- **From a template…**: pick a starting point (Daily summary, Review new files, Approve then send, Two-agent pipeline), name it, and choose an agent for each step. Nandi saves it and opens its graph. If you have no agents yet, it offers a starter pack first.
- **Describe it in Ask**: say what you want in plain words and let Nandi write it.
- **Import a workflow file…**: bring in a YAML file from elsewhere.

Under the hood, workflows are YAML files in `~/.nandi/repo/.nandi/workflows/`. The file name must match the `id`:

```yaml
id: daily-report
name: Daily Report
description: Reads the day's papers and posts a short report.
steps:
  - id: research
    agent: arxiv-researcher
    input: "Find today's three most-discussed papers on AI agents"
  - id: summarize
    agent: paper-summarizer
    input: 'Summarize each in one paragraph: {{research.output}}'
    critical: true
  - id: publish
    agent: shell-helper
    input: 'Write this to report.md: {{summarize.output}}'
    requires_approval: true
```

- `{{step.output}}` is an earlier step's final reply. That's the only way data passes between steps: each agent has its own working folder, so agents can't share files.
- `requires_approval: true` ("Asks you first") pauses the run before that step's message is sent.
- `critical: true` puts the step ahead of other background work in the agent's queue.
- `description` is optional; it's what the workflow's Overview says it does.
- Unknown keys are errors, so typos fail loudly.

Save the file and it appears under **Automations → Workflows**, with how many steps it has, what starts it ("At 09:00, Monday through Friday", a webhook, or you), and how its last run went.

### Run it and follow along

![screenshot: Workflow graph with step details](placeholder)

Open the workflow and click **Run** (it's greyed out while a run is already going). The page has four tabs:

- **Overview**: what it does, how the last 30 days went, its steps with the agent for each, the triggers that start it (with the next scheduled time) plus **Add a trigger**, and recent runs. A run opens its request.
- **Graph**: the steps and the `{{…}}` links between them, with live status on each node. Click a node to open its panel beside the graph (drag the edge to resize it, × to close it). There you can change the step's agent, its message and **Ask me first** at any time. If a run is going, the change applies to the next one.
- **Runs**: every run, who started it (You, a trigger by name, or another Nandi), how long it took, and its transcript step by step. **Run again** asks first, because every step runs again.
- **Configuration**: the workflow file itself (advanced), with **Revert** and **Save**. Your draft survives switching tabs, and leaving the page with unsaved changes asks first.

When a run needs you, a banner at the top names the step and its agent, with **Approve** and **Reject** (Reject asks first, since the run stops there), or **Mark done** for a step a person does. When a run fails, the banner says where it stopped, with **Open request** and **Run again**.

Each step is an ordinary agent run, so it also appears in that agent's chat and run history. If an agent is busy, the step waits in its queue. Runs survive restarts: an interrupted workflow resumes where it was, and one waiting for approval just keeps waiting.

### Steps that wait for a person

Some work needs a human in the middle: signing off a draft, tagging a laptop, uploading a contract. Make that step manual:

```yaml
- id: laptop
  step_type: manual
  instructions: 'Tag the laptop for {{accounts.output}} and paste the tag here'
  deadline_hours: 48
```

When the run reaches it, the workflow pauses and waits, for minutes or for days. You get a notification, the Dashboard and the companion say "Onboarding needs you", and **Work → Waiting on me** shows a card with the instructions and the due date. Type your reply (and attach files if you need to), then click **Complete step**. Your reply becomes `{{laptop.output}}` for the steps after it, and the workflow carries on. Quitting Nandi doesn't lose a waiting workflow: it's still waiting when you come back.

---

### Steps that ask a colleague

A step can ask someone else's Nandi and wait for the answer:

```yaml
- id: evidence
  ask:
    find: 'SOC 1 evidence'
    input: 'Please send the FR-07 approval log for October: {{scope.output}}'
    due: 5 days
```

Name one of `find` (whoever on your network lists that topic), `person` (a colleague's device ID) or `capability` (anyone who offers it, such as `review-pr`). With `find`, the run first shows you who it will ask ("Ask Lena Fischer: …") and waits for your approval. Then it sends the request and waits, for days if needed, even across restarts. The answer becomes `{{evidence.output}}` for the next steps. If they decline, or nobody answers by the due date, the step fails (or the run carries on, with `on_failure: continue`). One person or capability per step: to ask several people, use several steps.

---

## 7. Triggers

<!-- studio-guide: set-up-a-trigger -->

A trigger starts an agent run (or a workflow run) when something happens. Create one from **Automations → Triggers → New Trigger**, or from a workflow's **Add a trigger**; it's saved as a YAML file in `~/.nandi/repo/.nandi/triggers/`.

The list shows each trigger with an on/off switch, when it fires in plain words, what it runs, and when it last fired (click the time to open what that firing started). The ⋯ menu has **Test…** and **Delete…**. A trigger's page has two tabs: **Settings** (Revert and Save; leaving with unsaved changes asks first) and **Firings**, where every firing shows its result and what it received, and opens the work it started.

| Source       | Fires when                                                  |
| ------------ | ----------------------------------------------------------- |
| **Schedule** | A cron schedule's next time comes round (your local time)   |
| **File**     | A file in a watched folder is created, modified, or deleted |
| **Webhook**  | Something POSTs to the trigger's URL with its secret        |
| **Device**   | A device POSTs an event for its device ID                   |
| **Event**    | An agent run finishes, or a workflow finishes or fails      |

### Scheduled

```yaml
id: daily-digest
name: Daily AI News Digest
enabled: true
source:
  type: cron
  config:
    schedule: '0 9 * * 1-5' # 9:00 on weekdays
target:
  type: agent
  id: arxiv-researcher
input_template: |
  Find AI infrastructure news from the last 24h and summarize the top 5.
rate_limit_seconds: 3600
```

In the form, pick a **Preset** or choose **Custom** and type a cron expression. Nandi reads it back to you in words ("At 09:00, Monday through Friday (your time)") with the next three run times, checked by the same scheduler that runs it. If Nandi can't read the schedule, it says so and Save stays off. Schedules use standard cron fields, so `1-5` means Monday to Friday.

**Priority** is High, Normal or Low. High goes ahead of other background work; Normal uses the default for that kind of trigger. An exact number (1–100) is under **Advanced**, along with the rate limit.

### File-based

A file trigger watches one folder (it must exist when you save the trigger), optionally filtered by file-name patterns, with a 2-second debounce per file. In `input_template`, `{{payload.path}}` is the file that changed. Good for "summarize every PDF I drop into `~/Inbox`".

### Webhook

Saving a webhook trigger generates a secret and shows it **once**. Copy it then. **Generate a new secret** makes another, after warning that the old one stops working at once. The trigger's page shows its address (Copy stays off until Nandi's local server is up, so you never copy half an address) and a ready-made command to try it. Callers send the secret in a header:

```bash
curl -X POST http://127.0.0.1:17463/api/triggers/webhook/new-orders \
  -H "X-Trigger-Secret: $SECRET" -H "Content-Type: application/json" \
  -d '{"order": {"id": 42}}'
```

The desktop app's web port is on `127.0.0.1` only, so webhooks work from your own machine. To receive them from other machines, run Nandi with `--server` (all interfaces, port 17463). Payloads over 256 KB and more than 60 requests a minute are refused.

### Guard rails

Every trigger has **Test…** (works even while it's off). It warns that the test really runs the agent or workflow, lets you edit the test payload, and fires only when you click **Fire it now**. Every trigger also keeps its firings history. A trigger that fires more than 5 times in 10 seconds is disabled automatically, and `rate_limit_seconds` spaces out dispatches. An event trigger never fires on a run it started itself, so "retry on failure" rules can't loop. Secrets live in your keychain, never in the YAML file; a trigger pulled onto a new machine gets a fresh secret.

---

## 8. Library

**Library** has everything your agents can be given: agents and packs (agents plus workflows) from marketplace sources, the workflows packs bring, built-in tools, connectors, and pilots. It has three tabs: **Installed** (what's on this computer), **Browse** (everything your sources offer), and **Shared with me**. The chips **All · Agents · Workflows · Tools · Packs** narrow any tab and show counts; **Can do** keeps only what can do one thing (say, `find-papers`). Search works per tab.

### Cards and details

Every card shows what it is, who publishes it, one status (Installed, Running, Needs setup, Update available, Off…), one line of what it's allowed to do ("Reads files · Runs shell · Internet: 2 sites", or "No special access"), and one button. Click a card for its details: what it can do, what it's allowed to do, setup, which agents may use it, a pack's contents, and, under **Details**, the technical bits (ID, source, commit, port, keychain entries). **Verified** means the publisher signed the release and the code you'd install is exactly the signed commit.

### Install and allow

**Install** first lists exactly what the agent or pack will be allowed to do. **Install and allow** fetches the release, copies only the agent files (`skill.md`, `instructions.md`, README, icon, license, workflows; never scripts) under `~/.nandi/repo/.nandi/agents/marketplace/`, and records your consent, so the first run doesn't ask again. **Cancel** installs nothing. Unverified entries warn you first and say **Install anyway**. Installed agents are read-only; updates are never automatic: use **Check for update** in a card's **⋯** menu.

### Tools, connectors and pilots

Built-in tools (Run shell commands, Read files, Write files, List folders, Ask other agents) each have an **On/Off** switch; switching one off asks first, because agents that use it lose it. A connector that needs something shows **Set up**: its details have the settings (saved to `.nandi/connectors.yaml`, so they travel with your repo), a masked field for each key (saved to your keychain and never shown again; **Replace** or **Clear**, which asks first). **Start** stays off, with the reason, until setup is complete. Email isn't a Library connector: connect it in **Settings › Email** ([Connect your email](#connect-your-email)). Pilots install, start, and stop from their card; one that can't run on this computer says so.

**Shared folders.** The **Filesystem** connector holds the folders agents can be given: **Add a folder**, give it a short name, and choose **Read only** or **Read and write**. No agent gets a folder until you give it one: on the agent's page, under **Permissions**, turn on Filesystem and pick each folder (**Not shared**, **Read only**, or **Read and write** where the folder allows it). Writing a file still asks first. You can also ask in Ask: "give the report agent read access to docs". Agents that run in a container see the folders at `/agent/folders/<name>`.

<!-- studio-guide: use-the-browser -->

**The browser.** Two pilots let an agent use a real browser: **Browser** opens a Chrome or Edge window on this computer so you can watch, and **Browser (headless)** runs one in a sandbox with no window (it needs Docker). Install and start one in Library › Tools, list it under `pilots` in the agent's skill.md, and allow it for the agent. Each agent gets its own browser profile, so its sign-ins stay separate from yours. The browser opens the sites in the agent's **Internet access**. When it needs another site, Nandi asks you first: the card shows the agent, the site (with its raw address when the name has accents or other scripts) and why the agent wants it. Choose **Allow once** (this run only), **Always allow for** the agent (added to its Internet access), or **Don't allow**; tick **Include its subdomains** to cover `www.` and the like too. IP addresses, local names such as `printer.local`, other ports, and sites a policy rules out are refused without asking. Reading pages, taking screenshots (saved with the run's files) and opening allowed pages never ask. Clicking and typing, submitting a form, typing a secret, files and paying wait for your approval, unless the agent's pilot entry allows them (`allow: [interact, submit]`). A pilot entry can also hold plain `values` the agent may type and `secrets`: the agent types a secret's name and the browser fills in the value, which you save on the agent's **Permissions** (it stays in your keychain and the agent never sees it).

**Sites and accounts.** On the agent's **Permissions** tab, each browser lists its secrets with **Secret saved** or **Not set**, never the value. Type a new value and **Save** to set or replace one, or **Clear** to remove it. For an agent made on this computer, including Chief of Staff, **Add secret** takes a name like `ACME_PASSWORD` and the value: Nandi adds the name to the agent's skill.md and keeps the value in your keychain. An agent that came with a pack names its own secrets; you set their values. **Sign out everywhere** stops the agent's browser and deletes its profile (cookies, sign-ins and site data) after you confirm, so every site forgets it; your saved secrets stay. Browser profiles stay on this computer and are never backed up. Chief of Staff keeps what a site needs, without secrets, in My notes under `private/sites/` (a customer number, the menu path, which secret name to use). Nandi refuses to write a card number or a password line into your notes.

**Hand-over.** Some agents prepare a page in the browser and leave the last click to you: a social post typed into LinkedIn's composer, say. Nandi brings the browser window forward and your Inbox shows **Your turn: review and post**, with the draft. Change anything you like in the window and press Post yourself. Then answer in Nandi: **Done, I posted it**, **Skip**, or **Changed it** (paste the text that went out). The agent reads your answer and carries on; it waits up to a day. A policy can refuse browser controls outright, such as Post buttons on social sites or Like and Follow everywhere (`browser: { refuse: [...] }`); the agent is told why and hands over instead. The **Nandi Social** starter pack works this way.

<!-- studio-guide: connect-a-service -->

**GitHub.**

- Lets agents read pull requests and files, and post a review with inline comments. Posting always waits for your approval.
- Click **Sign in with GitHub** in its Setup (or in Settings › Account): enter the code GitHub shows you and approve. That one sign-in also works for Backup, teams, sources and publishing on github.com. If you'd rather not, paste a fine-grained token (Contents: read; Pull requests: read and write), or sign in with the GitHub CLI (`gh auth login`).
- The calls run on your computer, so the token never reaches an agent's container.

**AWS.**

- Lets agents read CloudWatch alarms and Logs (Insights queries and recent events) and AWS Batch jobs, and restart a failed job after your approval.
- It uses the AWS CLI on your computer with the **profile** and **region** set in its Setup. Keys, SSO and `credential_process` profiles all work, and credentials stay on your computer.

**S3 Backup.**

- Backs up the folders you add under **Folders to back up** in its Setup to your own S3 bucket, as Glacier Deep Archive: about $1 per TB a month. Restores take hours, so think of it as insurance, not a sync folder.
- Changes are backed up within 15 minutes, and everything again every night at 2:00. Deleting a file on your computer never deletes its backup.
- With the **Personal Backup** pack, run **Set up S3 backup** in Automations (or tell Backup Keeper "set up my backup"). It can open the AWS sign-up page, then creates a private bucket and a user that can only upload to it, each after your approval. That user's key is saved on your computer and never shown to the agent.
- Saving Setup with a bucket and folders adds the triggers `s3-backup-<folder>` and `s3-backup-nightly` in Automations, where you can pause them.

**Other services (MCP servers).**

- Any service with an MCP server (Outlook, Slack, Jira, a database…) can be added as a connector. The easiest way is to ask in Ask: "add the Microsoft 365 MCP server", or paste the `mcpServers` snippet from the server's README.
- Ask shows what will run, which hosts it may reach and which tools run without asking. It saves the connector once you approve.
- Keys and tokens go into its Setup, never into the chat. Servers that only sign in through the browser aren't supported yet.

### Fork

To change an installed agent, **Fork** it from the **⋯** menu. The fork is a normal agent in your repo with a new ID; it no longer receives updates. Anything else that still refers to the original (a pack's workflow, say) is listed so you can point it at the fork.

### Sources

**Browse** shows when your sources were last updated (amber when it's over a day), **Refresh**, and **Sources ›**, which opens **Settings › Sources**. There you add marketplace sources (a URL plus a token stored in your keychain, or **Use my GitHub sign-in**) and capability sources. **Create my community registry** makes a private `nandi-community-<you>` repo you can invite collaborators to. To share your own agent publicly, use **Publish** in its header: Nandi tags the release in your agents repo and opens a pull request against the registry.

---

## 9. Teams and collaboration

Teams are for 5 to 20 people who trust each other. There's no Nandi server: a team is a private `nandi-registry` repo in a GitHub organization.

### GitHub-org teams

Open **Network › Teams** and click **New team**. Pick one of your GitHub orgs; Nandi creates a private `nandi-registry` there if it doesn't exist, or **Join existing** if a teammate already did. Membership and roles (Owner, Admin, Member) come from the org.

**Share agent** pushes the agent's folder to a `nandi/share-…` branch in the team registry; Nandi says which branch, with **Copy branch**. A team admin must merge it, and until then the agent shows **Waiting for a team admin to merge** on the Team page. Teammates see it under **Shared with me**; your shared agents are under **Shared by me**, with call counts.

### Entra ID groups

Organizations on Microsoft 365 can back teams with Entra ID groups instead of GitHub org membership. An admin enters the tenant and app (client) ID once in **Settings → Entra ID**; then each person signs in with Microsoft using a device code, the same way as GitHub. Groups can be mapped to teams, and **Sync from Entra ID** on the Team page keeps membership current. GitHub still stores the agents; Entra decides who is on the team.

### My notes

<!-- studio-guide: my-notes -->

**My notes** (left menu) is what Chief of Staff knows about your work: your role, what you own, deadlines, who to ask, runbooks. They're plain Markdown files in one folder (shown at the top), so you can also open them in any editor.

- **Chief of Staff keeps them.** With **Let Chief of Staff keep them up to date** on, tell it things as they happen ("the walkthrough moved to Nov 19", "Lena is now backup for billing"). It notes them in the right file without asking each time, and says where. It also raises what's coming up: overdue items, deadlines this week, and notes that disagree. It does this while you're talking with it, not in the background (see [What runs on its own](#what-runs-on-its-own-and-what-doesnt)). New users: open Chief of Staff and say hello. It asks a few questions and writes your first notes.
- **You can edit them too.** Pick a note and choose **Edit** (Markdown: `#` headings, `-` lists, `|` tables). Ctrl+S saves. Use **New note** for anything else, such as `runbooks/etl`.
- **What others see.** Teammates' agents can ask your Chief of Staff about your work, and it answers from these notes. Anything in a `private/` folder is never used for that. What you own is published from **Network › Sharing › What you offer**; Chief of Staff suggests updating it when your notes change.

### Connect your email

Chief of Staff can keep My notes up to date from your email, so you don't have to write them. Open **Settings › Email** and choose **Connect Outlook** (Outlook and Microsoft 365) or **Connect Gmail**, then sign in in your browser.

- **What it reads.** New mail in your inbox and sent items, every 30 minutes by default (**Check every**: 5 minutes to 2 hours). Newsletters and notifications are left out. Add senders or domains under **Always include** if their mail matters even when it looks like a newsletter. The first check reads the last 14 days.
- **What it does with it.** Chief of Staff notes what you're asked to do, what you promised, deadlines, decisions and who owns what. Each note says which email it came from: sender, date and subject. Edit or delete any note; it won't add a deleted one back.
- **What it never does.** Nandi never sends, moves, deletes or marks your mail read, and keeps no copy of it: only where it got to, and the notes. Your sign-in stays in this computer's keychain and never goes into a container. Text in an email is never treated as an instruction.
- **Older mail.** **Also read older mail in the background** (at setup and in **Settings › Email**): **Last 3 months**, **Last year**, **Everything** (the default) or **Don't**. Before it starts, Nandi estimates how many emails and Chief of Staff runs that is. It reads a batch of up to 60 emails every 10 minutes, newest first, only when Chief of Staff isn't busy with new mail. Past deadlines are noted as history, not to-dos. The card shows "Reading older mail: back to <month>, N emails so far" with **Pause** and **Stop**, then "Older mail read".
- **Your controls.** **Check now**, **Pause**, and **Stop reading my email**, which signs out and forgets where it got to. Your notes stay. With no mailbox connected, Home shows **Connect your email** with the buttons to do it.
- **If sign-in stops.** Some companies need an IT admin to approve Nandi first. Nandi says so and gives you a link to send them. Gmail test sign-ins end every 7 days: choose **Reconnect**.

### Auto-approve rules

Mail and teammates' requests that Chief of Staff screens wait in Home's **Waiting for your OK**. **Settings › Auto-approve** lets some of them through without a click. **Add rule** takes an **Email domain** (`chase.com`, which also covers `alerts.chase.com`), an **Email address** (`billing@acme-energy.com`) or a **Teammate** (picked from your relay's members; without a relay it says to connect to it, and the other two still work). Each rule has actions you tick: **Start work** and **Add news to work** for mail, **Run their requests** for a teammate. Untick one, or click the bin to remove the rule.

- **Screening still runs first.** Only a clean item passes. A suspicious item still waits for you, and a blocked one is still refused.
- **Mail must be verified.** A mail rule only matches mail whose sender passed the sender check (DMARC), so a forged "From: chase.com" never passes.
- **Teammates.** The rule matches that person's Nandi (signed) and applies to their requests.
- **You can see it.** An item a rule approves is noted in its thread ("Approved automatically (mail from chase.com)") and shows in **Since you were away**.

On a clean email item that waits, **Always allow mail from <domain>** adds a domain rule with that item's action, after a confirm. Teammate items don't have that button: add them in Settings.

### Finding who is responsible

List what you're responsible for in **Network › Sharing › What you offer** (one per line). Everyone on your relay can then find you by topic, even if your profile is private.

The **Chief of Staff** agent comes with Nandi (first under Agents) and works with you and your team:

- **Who is responsible:** answers "who is responsible for the SOX audit?" from what people published.
- **Asking and assigning:** asks people or their agents, and assigns work, each after your approval.
- **Tracking:** shows what you're waiting on and what people asked you.
- **Answering for you:** answers questions about your work for others. Work someone assigns you waits for your approval in your Inbox.

**Browsing websites.** Chief of Staff can browse for you in your chats: finding or checking something on a site, filling a form, downloading a statement. It uses the **Browser** pilot, so you watch every step in a Chrome or Edge window (install and start it in Library › Tools). It opens only the sites you allow: when it needs a new one, Nandi asks you first. It says what it will do before each change, waits for your approval to submit, upload or pay, and hands the window to you for sign-ins, CAPTCHAs, payments and passwords. It never browses when it reads your email or calendar on its own.

**Its own steps in your work.** When Chief of Staff plans work, it can take one step itself, for example "pay the bill on acme-energy.com" in its browser. That step runs in the task's thread (or the work's own conversation), never in your chat with it, and the work goes on to the next step when it's done. Anything new it wants to start from there waits in **Waiting for your OK**.

**Save as workflow, offered.** When work finishes well and the same work finished well before (last month's bill, last week's report), Chief of Staff offers to save it as a workflow: a note in the task's thread and **Save as workflow** on the work there and in **Since you were away**. It opens the draft for you to check; your parts (signing in, paying) are manual steps in it. Nothing is saved until you click **Save workflow**.

**Playbooks.** Chief of Staff comes with playbooks for the work it does most: `web-task`, `pay-a-bill`, `sign-in-and-accounts`, `delegate-work` (do it itself, give it to an agent or a workflow, or ask a person), `build-a-workflow`, `email-triage`, `meeting-prep` and `occasions` (birthdays and anniversaries, with a message or post for you to approve). They're yours to change on its **Playbooks** tab. Nandi adds them once; a playbook you edit or delete stays that way.

**Playbooks from repeated work.** When Chief of Staff does the same kind of work itself a second time, it adds one line to `suggestions.md` in your notes: `- [ ] Add a playbook "pay-acme" (from 2 similar tasks)`. Tick it (on My notes) or say yes, and it drafts the playbook for you to approve, as above. It never repeats a suggestion you dismissed.

**Assigned work stays on your list.** Say "Assign Lena: send the evidence list, due Friday". Your Chief of Staff tells you it's assigned and moves on; you don't wait. Once Lena approves, her Inbox shows "Dev assigned you this" with the due date (or how overdue it is), a note field, **Done** and **Decline**. It stays there for days and across restarts. When she clicks **Done**, her note comes back to you, even if you were offline then. Ask "What am I waiting on?" to see what's open, what's due and what's done.

### Sharing agents for delegation

<!-- studio-guide: network-and-delegation -->

Sharing a folder lets teammates install your agent. **Delegation** lets their agents call yours while it runs on your machine. In your agent's **Permissions** tab:

- **Accepts tasks from other agents**, plus the capabilities it serves (e.g. `summarize-paper`) and how many tasks at once.
- **Can call other agents** gives your agent the `delegate_to_agent` tool. Each call asks you first.

**Network › Sharing** has five tabs: **What you offer** (the capabilities your gateway exposes, hides or holds for approval, and "About this Nandi"), **Who may call you** (callers as a list, or the trust graph), **Incoming requests**, **Shared** and **Capabilities**. In the graph, click a call line to set the policy for that caller: **Allow**, **Ask**, or **Deny**. **Delegation history** lists every task your agents asked for and served, with duration and tokens.

### Shared with me and Shared by me

Both lists group agents by owner ("Alice · Agents: 3"). Click the arrow next to an owner to fold their agents away; **View profile** opens their member profile, and **Trust** opens their trust switches.

### Everyone on your company's relay is trusted

On a relay that needs a token (your company's own), only people the company enrolled can join. So Nandi trusts them: their requests come straight in, without marking each one **Trusted peer**. You can still pause or block anyone, and work assigned to you still waits for your approval in your Inbox. To trust people one by one instead, turn off **Trust everyone on this relay** in Network › Connection. On an open relay (started with `--open`), trust stays one by one.

### Trust requests

When someone who isn't trusted yet tries to send you work and your policy is **Ask**, the attempt isn't lost: it shows up as a trust request in your Home **Inbox** and at the top of **Network › Sharing** ("Sam wants to send you work · tried 2 times"). **Trust** lets their next attempt through; **Deny** (optionally **and block**) turns them away.

### Members and trust

**Network** lists everyone connected to your relay, online first, with what their gateway offers; filter to **Online** or **Recently active**. Open someone to see their profile, with a **Can {name} receive from me?** check that lists what's missing (on the relay, listed, accepts tasks, trusts you…) and a message you can copy and send them: their gateway (their NANDI.md), capabilities and agents, recent delegations between you, and trust. Trust has three switches, also in the profile's header:

- **Trusted peer**: adds them to the trusted peers in your NANDI.md.
- **Pause delegations**: your gateway turns their tasks away and you stop sending them work, until you switch it back.
- **Block this member**: their tasks are rejected the moment they arrive and none of their agents are used. To block just one agent, use **Block** on it in **Network › Sharing › Capabilities**.

### Groups

**Network › Groups** lists every group on your relay: its name, what it's for and how many people are in it. Nobody is in a group until they join. Anyone can **Create group** (and is its first admin); to join one you need an invite: **Join with code** with a code a member gave you, or **Accept** an invite sent to you, shown at the top of the page.

Every group has a **policy**: what members may have done for each other, and how (for example, every request waits for approval, only some capabilities, at most 20 requests a day, only during working hours). The new-group form starts from a template whose comments explain each line. An admin signs the policy with their device's key, and your Nandi checks that signature: a group shows "Verified, signed by Alice, version 3". If a policy's signature doesn't check out, the group shows "This group's policy couldn't be verified" and your Nandi refuses work from that group until a good version arrives.

In a group you're in, pick **My access level**:

| Level           | What members of the group may have your Nandi do                         |
| --------------- | ------------------------------------------------------------------------ |
| Fully automated | Run their requests without asking you, when the group's policy allows it |
| Ask first       | Every request waits for your approval (the level you start with)         |
| Questions only  | Ask you questions, answered from your notes; everything else is refused  |
| Send me info    | Send you things to read; questions wait in your queue and nothing runs   |
| None            | Nothing                                                                  |

The stricter of your level and the group's policy applies: you can be stricter than the group, never looser. If you share several groups with someone, the strictest of them applies. People who share no group with you are handled as before, by your relay's trust rules.

Admins can rename the group, change its policy (**Edit policy**, then **Sign and save**), make other admins and remove members; every member can invite someone they can see or create an invite code with a number of uses and an expiry, and anyone can **Leave**. Your copies of your groups' policies and your access levels are files in `~/.nandi/repo/.nandi/groups/`, backed up with the rest of your repo.

**A relay that shows only your groups.** Some relays (a public one, rather than your company's) are set to show you only the people you share a group with. People then says so, and asking someone you share no group with answers "You don't share a group with them on this relay".

### Handing a task to a teammate

If you're serving a task and are swamped, pass it on. In the agent's task strip, **Reassign** lists teammates who are online and serve the same capability; pick one. The task moves to them without the person who asked having to send it again: their delegation thread says "Reassigned to Bob by Alice" and links to the new attempt, and the result reaches them as usual. A task never goes back to someone who already had it.

---

## 10. Relay and remote delegation

Delegation between agents on the same machine needs nothing extra. To reach a teammate's machine, both instances connect to a **relay**: a small WebSocket router that lets instances find each other and pass tasks, even behind NAT. It stores nothing; task inputs and outputs only pass through.

> **Your company runs its own relay.** One relay is one company: everyone enrolled on it can find everyone else. It needs a secret, so only people your company enrolls can join, and it can serve `wss://` (encrypted). A relay started with `--open` has no sign-in and anyone who can reach it can see and call the agents you share: use that only to try things on one machine or a network you trust.

### Run a relay

On one machine everyone can reach, with a secret and a join code for your staff:

The relay is its own program, `nandi-relay` ([vaultdbai/nandi-relay](https://github.com/vaultdbai/nandi-relay)); Nandi itself doesn't run one.

```bash
NANDI_RELAY_SECRET=<long random value> NANDI_RELAY_JOIN_CODE=<code for staff> \
  nandi-relay --host 0.0.0.0 --port 8787 --tls-cert relay.pem --tls-key relay-key.pem
```

Allow port 8787 through that machine's firewall. Without a certificate it serves plain `ws://`; put it behind your TLS proxy, or keep it on a LAN you trust. To try it on one machine: `nandi-relay --host 127.0.0.1 --open`. A relay serves up to 20 people free; a licence file from VaultDB.ai (`--licence`) adds more, and past the limit a new person's Connection view says why they can't join. When someone leaves, the relay's operator revokes their device (`/relay/revoke`, see `docs/relay-testing.md` in nandi-relay), and it's off the relay at once and frees their seat.

### Join a company relay

![screenshot: Relay screen](placeholder)

Whoever runs your company's relay gives you two things: its address (`wss://relay.example.com/relay`) and a **join code**. A teammate who has joined can send you both in one invite.

1. Open **Network › Connection**. Under **Join a company relay**, enter the **Relay URL** and the **Join code**. With an invite, click **Paste invite** (or paste the whole message into the address field): it fills both.
2. Click **Join**. **Test connection** only checks that the address answers.

Nandi joins once: the relay checks the code and gives this Nandi its own key, which stays on this computer. From then on Nandi reconnects on every launch. The join code is never shown again once you've typed it. **Disconnect** leaves the relay; joining the same relay later reuses your seat.

If joining doesn't work, Nandi says why, and everything else keeps working:

| What you see                        | What to do                                                                      |
| ----------------------------------- | ------------------------------------------------------------------------------- |
| Couldn't reach a relay at …         | Check the address, your network or VPN, or try later                            |
| This relay needs a join code        | Ask whoever runs it                                                             |
| That join code wasn't accepted      | Check the code with whoever runs the relay                                      |
| … has no free seat. The relay says: | The relay's own words follow (a free relay serves 20 people). Tell its operator |
| … speaks relay version …            | Update Nandi, or ask the operator to update the relay                           |
| This Nandi joined … before          | Ask the operator to issue this Nandi's key again                                |

**Invite a teammate.** **Copy invite message** (on Network › Connection, and on People while you're alone on the relay) copies a short message with the relay's address and, if you typed one, its join code. Share it only with teammates. It never holds your own key.

A relay with no sign-in (`--open`) needs no join code: leave the field empty.

Below it, **Make this Nandi reachable** is a checklist of everything others need before they can send you work: connected to a relay, listed, confirmed by the relay, agents that take requests by name, who may call you, approvals, and what you show others. Each row that isn't done has one button that fixes it. **How networking works** (top right, on every Network tab) explains the terms; dotted words have a one-line tooltip. Your device ID is under **Details**.

Where the URL comes from, highest priority first:

1. The `NANDI_RELAY_URL` environment variable (handy for testing; the screen says so when it's in use)
2. The URL you saved, kept as `relay_url` in `~/.nandi/config.json`
3. Nothing: Nandi stays disconnected

### Delegate

When connected, the Relay screen lists the agents online, grouped by capability. **Call** sends one task with JSON input to a specific agent; the result comes back to you. Your agents with **Can call other agents** can also delegate on their own: they'll pick a provider for the capability (this machine first, then the least busy), after you approve the call. The owner's trust policy decides whether the task runs on their side.

### Following a request

<!-- studio-guide: requests-and-work -->

Every task sent or received opens on its **Request** page (from Work, the Gateway queue, Delegations or a conversation's **Open as request**). It shows where the task is, step by step: Received → Allowed → Routed → the agent or workflow doing it → Delivered for requests others send you; Sent → Delivered → Accepted → Working → Done for yours. If a provider timed out and Nandi tried another, each attempt is listed under the Working step.

When something is stuck, one banner says why and who has to act, with the button that fixes it: **Approve**, **Trust {name}**, **Turn on for {agent}**, **Add a model**, **Try next provider**. Buttons that affect someone else (Reject, Trust, Cancel, Stop) ask first. Raw IDs and the system's own wording are under **Details** and **Activity**. A run that served a request links back to it from Run Detail ("Part of a request").

**Your own asks are Requests too.** When the ask box answers with a plan, **Run plan** saves it as a Request and opens its page, the same page as work from others. Nandi carries out the steps in order, links each one to the run, workflow run or delegation doing it, and picks up where it was if you restart. A plan that only points at people or manual steps starts without asking; anything that messages an agent, runs a workflow or delegates is shown first. Steps that wait on you have **Mark done**. When it's finished, **Do this again** runs the same plan as a new Request, and **Save as workflow** drafts a workflow from its steps: check the file, change anything, and save.

### One LLM setup for the whole team

If your team runs models through AWS Bedrock, Azure AI Foundry, or a LiteLLM gateway, put the setup in the repo once and everyone who clones it gets it. Create `.nandi/llm.yaml` in your agents repo:

```yaml
providers:
  - id: bedrock-primary
    kind: bedrock
    region: us-east-1
    profile: acme-dev
routing:
  default: bedrock-primary
```

The file holds no secrets: Bedrock uses the AWS CLI profile you name (sign in with `aws sso login`), and keys stay in your keychain or environment. Your own environment variables (`AWS_PROFILE`, `AWS_REGION`, `NANDI_LLM_MODEL`) still win over the file, and the file wins over this machine's settings. **Settings › Models › Advanced** shows what the repo provides and where each value in effect comes from; **Sync** re-reads the file after you pull changes.

---

## 11. Local models

A Nandi-managed local model server is **coming soon**: a container (or, on macOS, a native Ollama) that agents can use with no API key and no data leaving your machine. See the [roadmap](https://github.com/vaultdbai/nandi/blob/main/docs/roadmap.md) (placeholder link) for status.

Until then there is an experimental shortcut: if you run [Ollama](https://ollama.com) yourself, an agent with `model: ollama:llama3.2` (or any model you've pulled) talks to it at `http://localhost:11434`. Expect weaker tool use than hosted models, and pick a model that supports tool calling.

---

## 12. Troubleshooting

### Common failures

| Symptom                                                                          | Cause and fix                                                                                                                                                                                                                                              |
| -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Agents need Docker to act. Set up" on an agent page; agent answers in text only | Docker isn't running or the tool sandbox image is missing. **Set up** opens Settings › This computer with the steps for your computer; download the image in Settings › Models & runtimes (see [Installation](#2-installation)). The next message retries. |
| "Add your DeepSeek API key to start chatting"                                    | No key for the agent's provider. **Settings › Models**.                                                                                                                                                                                                    |
| A new agent doesn't appear                                                       | `skill.md` must be at `~/.nandi/repo/.nandi/agents/<id>/skill.md`, and the folder name must be a valid ID. Invalid files are reported; the last good version stays live.                                                                                   |
| An agent seems stuck                                                             | Check the strip above its chat: it's probably waiting for an approval or working through its queue. **Stop** cancels the current turn.                                                                                                                     |
| A marketplace agent won't run                                                    | It needs consent: **Review and allow** in its banner.                                                                                                                                                                                                      |
| Sync now fails with a conflict                                                   | Nothing was changed. Resolve the listed files with git in `~/.nandi/repo`, then sync again.                                                                                                                                                                |
| A webhook returns 401 / 404 / 429                                                | Wrong secret / trigger disabled or unknown / over 60 requests a minute.                                                                                                                                                                                    |
| Relay says Reconnecting                                                          | The reason line shows the last error. Usually a firewall, or a relay started with `--host 127.0.0.1`. Use **Test connection**.                                                                                                                             |
| A run stopped early                                                              | It hit `max_turns` or `max_cost_usd`. Raise them in `skill.md` if that was legitimate.                                                                                                                                                                     |

To check an agent's prerequisites (Docker, image, database, API key, `skill.md`) from a terminal, run the app binary with `verify --agent <id>`; it exits non-zero and names what's missing.

### Where things live

| What                             | Where                                                                                                                            |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Your agents, workflows, triggers | `~/.nandi/repo/.nandi/` (git)                                                                                                    |
| Run history, approvals, queue    | `~/.nandi/local/state.db`                                                                                                        |
| Audit logs                       | `~/.nandi/local/logs/`: `llm.log`, `egress.log`, `triggers.log`, `pilots.log`, `connectors.log`, `delegations.log`, `verify.log` |
| Agent working folders            | `~/.nandi/local/work/<agent-id>/`                                                                                                |
| Deleted agents, replaced repos   | `~/.nandi/trash/`                                                                                                                |
| App log                          | macOS: `~/Library/Logs/<app id>/`. Windows and Linux: run Nandi from a terminal to see it                                        |
| A container's own log            | `docker logs nandi-agent-<tag>-<agent-id>` (`docker ps` shows the full name)                                                     |

Set `NANDI_HOME` to use a different folder than `~/.nandi` (useful for a clean test profile).

### How to reset

From least to most drastic:

1. **Rebuild the index.** Quit Nandi and run it with `rebuild`. This re-reads every file in the repo into the local database. Run history is kept. Add `--force` to skip files that don't parse.
2. **Restart a container.** Use the container controls in the agent view (Stop, then Start), or `docker rm -f nandi-agent-<tag>-<id>`. The next message starts a fresh one.
3. **Reset local state.** Quit Nandi and move `~/.nandi/local` aside. Your agents are safe (they're in the repo); run history, approvals, and logs are gone.
4. **Start over.** Quit, then move all of `~/.nandi` aside. Nandi starts at setup again (your name, a model, a network, starter packs). To get your agents back, set up your git remote in **Settings › Backup** and restore from it. Provider keys stay in the OS keychain; remove them in **Settings › Models & runtimes** if you want a clean slate.

---

## 13. Security model

Nandi assumes the model can be wrong or manipulated, so agents get as little as possible by default and you widen access explicitly.

### What runs where

| Runs on your machine, outside any container       | Runs inside the agent's container     |
| ------------------------------------------------- | ------------------------------------- |
| The app, the orchestrator, model calls            | The agent's tools (`shell`, files, …) |
| The egress proxy, pilots, connectors              | Nothing else: one container per agent |
| Your keychain secrets, git sync, the relay client |                                       |

### What the container sees

- Its own folder, read-only, at `/agent/skill`, and its working folder, read-write, at `/agent/workdir`. Nothing else from your disk.
- No API keys. Model calls go through the host's proxy, which adds the key on the way out.
- No direct network. Requests go through the **egress proxy**, which allows only the hosts in the agent's `network` list and logs every request (allowed or denied) to `egress.log`. Redirects aren't followed, so an allowed host can't bounce the agent somewhere else.

```yaml
network:
  allow:
    - host: api.github.com
      methods: [GET] # read-only
    - host: '*.example.com' # subdomains only
```

- The container runs as a non-root user with every Linux capability dropped, no privilege escalation, and memory and process limits.

### What never leaves the machine

- **Secrets.** LLM keys, AWS keys, the GitHub token, trigger secrets, and registry tokens live in the OS keychain. Files in your repo hold references only (`nandi.llm.deepseek`), so the repo is safe to push, and pasting a key into a config file fails validation instead of being committed.
- **Run history and logs.** `~/.nandi/local` is never synced. The LLM log records model, tokens, and cost, not prompt text; the trigger log records metadata, not payloads; the delegation log records who called whom, not inputs or outputs.

What does leave: model calls to the provider you chose, network requests you allowed, git pushes to your repos, and (if you connect a relay) your shared agents' cards and the tasks they're sent.

### Other guard rails

- Agent output is untrusted. Chat messages render Markdown with no HTML and no remote images.
- Marketplace agents are read-only, can't run before you consent to each capability, and install with scripts stripped out.
- Pilots, which can control your OS, are host-side only (a container never gets a pilot's address), need per-capability consent, and log every call to `pilots.log`.
- The web UI (`--server`) needs an access token; webhook and device receivers need their trigger's secret.
- The relay is the weak spot today: LAN only, open, and unencrypted. Treat anything you share through it as visible to everyone on that network.
