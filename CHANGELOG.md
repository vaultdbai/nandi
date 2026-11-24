# Changelog

What changed in each Nandi release, grouped by what you notice. The release notes for each version are in [docs/release-notes/](docs/release-notes/).

## 0.3.0 (2026-11-24)

Speaks relay protocol 8 and still works with relays at protocol 7.

### New

- **Every Gmail and Outlook account.** Connect as many as you have. Each account keeps its own sign-in and read position. Settings › Email lists them all, with **Add another account**. Your first account stays this Nandi's identity. Existing connections move over on first start: nobody signs in again and no mail is read twice.
- **Files and calendar with each account.** Connecting an account asks once for mail plus your Drive or OneDrive and your calendar, as you tick them. Each account's row can add or drop them later. Read-only, always.
- **One morning brief across all your inboxes,** grouped by mailbox. Every email Nandi reads says which mailbox it came from.
- **Agents know which mailbox.** An agent's access can cover every mailbox or only the ones you choose. A yes to an agent covers the mailboxes you had then; one you connect later asks again.
- **Approve from your phone.** Pair your phone in Settings › Your devices with a QR code and a PIN. It sees your approvals and to-dos and can approve or decline, typing the PIN each time. Messages go sealed end to end through the relay, which can't read them. Anything that acts outside an agent's sandbox, payments, sending, new access and the browser typing stay on the computer.
- **The Lock box.** Keep API keys and passwords that agents can use but never see. Each key works only at the sites you bind it to, for the agents you allow, and is Open, Ask each time (the default) or Locked (Windows Hello). The egress proxy adds a key to requests to its sites; it never enters a container, a prompt or a log. Every use is on record.
- **My Nandi browser.** One browser profile per browser (Chrome, Edge) that you sign in to once with **Open my Nandi browser**. Agents you allow use your sign-ins there; saved passwords come through the browser's own sync and stay in the system keychain. Nandi never copies your everyday browser's passwords or cookies.
- **It gets better every week.** Once a week Nandi reads what you corrected and suggests up to three changes to its instructions, notes or workflows, each with its reasons. Yes applies it as one commit, Undo reverts it, and Nandi checks afterwards whether it helped. It never gives itself more access.
- **`agent.md`.** An agent is a folder with an `agent.md` (it was `skill.md`). Your agents and installed packs upgrade on first start, in one commit. Other agents see only an agent's name and description; its instructions load when it runs.
- **What do you already use?** Setup shows what your Claude, ChatGPT, Gemini, Copilot or company AWS/Azure plan can do in Nandi, honestly, with the next step.
- **Try free tokens** in Settings › Models sends one short message through the free tokens, even when your own model answers everything else.
- **A different model answered.** An agent's page and its chat say when a call used another model because the chosen one has no key.

### Changed

- Starter packs now need Nandi 0.3.0 (they ship `agent.md`). Nandi 0.2 asks you to update.

## 0.2.3 (2026-11-22)

### New

- **Free tokens to start.** A computer with no model of its own gets 100,000 free tokens on DeepSeek Flash, so Nandi works on day one without a key. Your own key or sign-in always comes first, and the free tokens switch off once your own model works. Connect Outlook or Gmail and all your computers share one allowance. The key stays on VaultDB.ai's server, never in the app.

## 0.2.2 (2026-11-22)

### New

- **VaultDB.ai's public relay.** No company relay? Join VaultDB.ai's public relay in setup or in Network › Connection, with the Outlook or Gmail you connected. No sign-in and no join code. Free for up to 10 groups, 50 people per group and 500 messages a day. Requests stay sealed end to end; the relay sees who asked whom, never what was said.
- **Setup starts on the public relay.** The network step has **Join the public relay** chosen. Nothing joins until you click it; **Continue** goes on without a network.

## 0.2.1 (2026-11-20)

The first release after 0.1.0. The `v0.2.0` tag was never released: its installer build stopped on a mismatched updater package, fixed here.

### New

- **Nandi, the default agent.** The Chief of Staff is now called Nandi. It has its own page, apart from your own agents, with Reset to defaults. Its morning digest and mail check ship as its own triggers.
- **Nandi's instructions stay current.** A new build updates Nandi's shipped files you never edited. A file you edited is kept, and Nandi's page offers the update with a diff: Take update or Keep mine.
- **Email first.** Setup asks for Outlook or Gmail. Nandi keeps My notes from your mail, reads older mail slowly in the background, and acts on what arrives through one screened queue, with a thread for every task, planned before you approve it.
- **Your email is your identity.** The connected email names this Nandi to your relay. There are no separate sign-in screens.
- **Inbox chats.** Every Inbox item opens a chat with Nandi. The Inbox is one calm line per item.
- **Decisions.** A Decisions page under Home keeps one record of what you decided, with filters and a Past decisions link in the Inbox.
- **Runs plan and keep evidence.** Every run plans first, finishes its plan and keeps the evidence. A run's Summary tab has an ask box.
- **Playbooks.** Say "remember how I do this" and an agent writes a skill file it loads only when needed. Nandi proposes playbooks and workflows from repeated work.
- **Work with teammates through your relay.** Requests and results are sealed end to end with per-device keys. The relay picks who gets a request, stores and forwards work, fails over, reassigns and hands over by calendar. The relay reads metadata only.
- **Groups.** Make groups with signed policies and access levels. The relay enforces each group's request rules: allowed capabilities, hours, daily limits and first-time approval. People can be filtered by group.
- **Sharing.** Choose which agents and capabilities your teammates see, and set your availability.
- **Connectors.** Install connector packages, grant access while an agent works, and use calendars and files. Studio can scaffold and publish a connector.
- **Browser.** A built-in Browser agent, "allow this site?" prompts, auto-approve rules, and handing the browser over to you.
- **Sandbox images are pulled.** Nandi downloads its signed images with size and progress instead of building them. Claude Code is fetched on first use.
- **Help without a model.** When no model answers, a built-in help guide covers the basics: installing Docker, connecting a model, joining a relay.
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
- A renewed Claude sign-in on this computer clears "Can't sign in" at the next check, and an agent's page checks again when it opens.
- An agent's status follows its live turn, so "Running" ends when the answer does.
- A relay that refuses this Nandi's token says calmly to get a join code and join again.

### Security

- **The default registry fails closed.** An unsigned entry, a bad signature, a branch ref or a tag moved after signing is refused. Updates come from the registry's signed entry, never from the publisher's tags.
- **The first-party key ships in the app.** VaultDB.ai's releases are checked against it. Other publishers' keys are pinned on first install, and a changed key is refused.
- The default registry wins its IDs, and a pack's workflows can't drive agents outside it.
- **Consent covers more.** First-use consent now covers connectors, shared folders and their access, and taking tasks from others. An update that adds or widens any of them, or widens a host's methods, asks again. Approving grants only what the dialog showed.
- **The relay instance token lives in the OS keyring**, not in `state.db`.
- **Relays start closed.** A relay refuses registration without a valid token; `--open` is for development only and says so loudly when it starts.
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
