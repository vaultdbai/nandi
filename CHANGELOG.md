# Changelog

What changed in each Nandi release, grouped by what you notice. The release notes for each version are in [docs/release-notes/](docs/release-notes/).

## 0.2.0 (2026-11-16)

### New

- **Nandi, the default agent.** The Chief of Staff is now called Nandi. It has its own page, apart from your own agents, with Reset to defaults. Its morning digest and mail check ship as its own triggers.
- **Nandi's instructions stay current.** A new build updates Nandi's shipped files you never edited. A file you edited is kept, and Nandi's page offers the update with a diff: Take update or Keep mine.
- **Email first.** Setup asks for Outlook or Gmail. Nandi keeps My notes from your mail, reads older mail slowly in the background, and acts on what arrives through one screened queue, with a thread for every task, planned before you approve it.
- **Your email is your identity.** The connected email names this Nandi to your relay and the free trial. There are no separate sign-in screens.
- **Inbox chats.** Every Inbox item opens a chat with Nandi. The Inbox is one calm line per item.
- **Decisions.** A Decisions page under Home keeps one record of what you decided, with filters and a Past decisions link in the Inbox.
- **Runs plan and keep evidence.** Every run plans first, finishes its plan and keeps the evidence. A run's Summary tab has an ask box.
- **Playbooks.** Say "remember how I do this" and an agent writes a skill file it loads only when needed. Nandi proposes playbooks and workflows from repeated work.
- **Work with teammates through your relay.** Requests and results are sealed end to end with per-device keys. The relay picks who gets a request, stores and forwards work, fails over, reassigns and hands over by calendar. The relay reads metadata only.
- **Groups.** Make groups with signed policies and access levels. The relay enforces each group's request rules: allowed capabilities, hours, daily limits and first-time approval. People can be filtered by group.
- **Sharing.** Choose which agents and capabilities your teammates see, and set your availability.
- **The public relay.** Join VaultDB.ai's public relay with your connected email, from setup or Network › Connection.
- **Connectors.** Install connector packages, grant access while an agent works, and use calendars and files. Studio can scaffold and publish a connector.
- **Browser.** A built-in Browser agent, "allow this site?" prompts, auto-approve rules, and handing the browser over to you.
- **Sandbox images are pulled.** Nandi downloads its signed images with size and progress instead of building them. Claude Code is fetched on first use.
- **Free trial.** Trial calls go through Nandi's trial gateway, 100,000 tokens per person. When no model answers, a built-in help guide does.
- **Get your computer ready.** A checklist checks Docker and the rest of what Nandi needs.
- **Install Docker for me.** When Docker or WSL is missing, one Go ahead updates WSL and installs Docker (winget on Windows, Homebrew or the official download on macOS). Windows or macOS asks for admin rights for each installer; Nandi itself never runs as admin. Nandi can also start Docker whenever it opens, if you say so once.
- **To-dos and reminders.** Nandi turns what you're asked to do, what you promised and what to remember into to-dos, from each email check, chats and events. They sit in the Inbox and the work tray in three groups (Do now, Check and close, Reminders), with due dates, overdue in red, and a bell for reminders. Mark one Done, or hand it to Nandi or any of your agents.
- **GitHub security check.** When you connect GitHub, Nandi checks your account and the organizations you own for two-factor sign-in, a public Backup repository, secret scanning and unprotected default branches, and turns each finding into a to-do with the fix. It reads settings only, never code or secrets.
- **Nandi tries its own failed work again.** A failed digest or mail check retries and cleans up after itself, and a missed schedule catches up. Email has a Read again.
- **Windows installers.** A per-user `setup.exe` that needs no admin rights, and an `.msi` for IT deployment.
- **Live screens.** Screens follow file changes in your gold copy, say who made them and refresh from disk.
- **Work tray.** A work tray on every page, and a slimmer Home.
- **Older relays keep working.** Nandi talks to a relay one protocol version older. A feature that relay can't do yet says "Your relay needs an update for this".

### Changed

- **Codex needs an OpenAI API key.** Codex runs in the container through Nandi's proxy, with Nandi's OpenAI key, or `CODEX_API_KEY` or `OPENAI_API_KEY`. A ChatGPT login and Azure OpenAI can't be used there.
- **Gemini CLI is refused in the container** until it has a proxy route. Each of its sign-ins would put a credential in the container.
- **Every harness session runs in the container**, with the same memory and CPU limits as native runs.
- **The free trial is the last resort**, after your own sign-ins and keys.
- **Relay groups replace git-registry teams.**
- **Capability definitions are local files only.**

### Fixed

- A run a crashed orchestrator left behind is failed, and startup closes crashed runs like a failed turn.
- An approval answered before its step reports back now resumes the run.
- A schedule skips a fire while its last run is still going.
- The Gmail cursor never moves past mail Nandi didn't read, and old mail and events are never read twice.
- A cut-off DeepSeek answer is an error, not a short answer.
- Relay work is acknowledged only once it is stored. A provider keeps its result until the relay has it, and a submission the relay didn't answer is sent again.
- Nandi stops retrying a relay that refused its token or keys.
- Large secrets are split to fit Windows Credential Manager, and a failed sign-in says why.
- A `uvx` connector starts in its container.
- The companion stays on the monitor you drop it on.
- Every screen shows the calm "connect to your relay" notice when there is no relay.
- An approval card for Claude Code or Codex shows the command it asks to run. It showed only "Run a command:".

### Security

- **The default registry fails closed.** An unsigned entry, a bad signature, a branch ref or a tag moved after signing is refused. Updates come from the registry's signed entry, never from the publisher's tags.
- **The first-party key ships in the app.** VaultDB.ai's releases are checked against it. Other publishers' keys are pinned on first install, and a changed key is refused.
- The default registry wins its IDs, and a pack's workflows can't drive agents outside it.
- **Consent covers more.** First-use consent now covers connectors, shared folders and their access, and taking tasks from others. An update that adds or widens any of them, or widens a host's methods, asks again. Approving grants only what the dialog showed.
- **The relay instance token lives in the OS keyring**, not in `state.db`.
- **Signing keys in the OS keychain.** `nandi keygen` and `nandi sign` take `--key keyring:<NAME>`, and `nandi keygen --import` moves a key file in, so publisher keys never have to sit on disk.
- **The same lockdown for every coding agent.** One test holds Claude Code and Codex to the same container checks on every change: no host credentials inside, and sites off the allow list refused.
- **Dependency fixes.** Every open Dependabot alert is fixed except `glib`, which moves with Tauri and is not reachable from Nandi.
- **Credentials stay on the host.** Bedrock calls are signed on the host, Claude Code keys stay on the host through an Anthropic passthrough, and the LiteLLM master key stays behind the proxy.
- The egress proxy refuses internal addresses after resolving a name.
- Audit logs are hash-chained and synced to disk, and Settings › Logs shows when one is degraded. Run events and approval decisions survive a `state.db` reset.
- A denied tool is refused before anyone is asked to approve it. A harness tool call is allowed by its ACP kind only.

### Upgrading from 0.1.0

- The upgrade is automatic. `state.db` upgrades in place, and Nandi keeps a copy first as `state.db.bak-v2`.
- Your Chief of Staff becomes Nandi with your edits kept, in one commit in your gold copy: "Upgrade from Nandi 0.1.0".
- If you joined a relay, the instance token moves from `state.db` to the OS keyring. You don't join again.
- Audit logs 0.1.0 wrote are kept as older entries (`<name>.pre-chain.log`), and a new chain starts.
- 0.2.0 speaks relay protocol 7. Update your relay to 0.2.0 too.
