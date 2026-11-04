# Task: Outlook (mail, calendar, availability, contacts) through an existing MCP server

**From:** owner idea 2026-10-24: a connector or pilot for Microsoft Outlook that reads email, schedules meetings, sees others' availability, drafts meeting invites, and reads the address book if allowed. "If it is too much work, look for something that already exists as MCP."
**Size:** S to M. No server of our own: Nandi already runs MCP servers as connectors (`kind: mcp` in `.nandi/connectors.yaml`; stdio servers run in a container, or on the host for agents whose policy says `connectors.run_on_host`).
**Not for the first demo** (scope).

## Existing MCP servers (found 2026-10-24, not yet vetted)

| Server                                            | How it reaches Outlook                                                       | Fit                                                                                                                  |
| ------------------------------------------------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Microsoft "Work IQ" Mail and Calendar (Agent 365) | Official, Graph                                                              | Preview. Needs Microsoft 365 Copilot licences and an admin-registered Entra app. Mail and calendar only, no contacts |
| `elyxlz/microsoft-mcp`                            | Graph, `uvx`, needs an Azure app ID                                          | Mail, calendar with `check_availability` (free/busy), OneDrive, contacts, several accounts                           |
| `ms-graph-mcp` (@developer.k)                     | Graph, Entra app with public client flows                                    | Modular: mail, calendar (free/busy, meeting suggestions), contacts, To Do, Teams                                     |
| `msgraph-mcp` (Glama listing)                     | Graph, device-code sign-in                                                   | 26 tools, dry-run previews before send                                                                               |
| `outlook-mcp` (COM automation)                    | Drives the classic Outlook desktop app on Windows, **no Azure registration** | Reads mail, calendar, free/busy. Windows and classic Outlook only (not "new Outlook"). Must run on the host          |

## Recommendation

1. **Windows demo, quickest:** the COM-based `outlook-mcp`.
   - Declare it as a `kind: mcp` connector with `run_on_host` for the Chief of Staff agent, since it needs the desktop Outlook.
   - No tenant setup, so it works with whatever account Outlook is signed in to.
2. **Cross-platform, for real use:** a Graph server such as `elyxlz/microsoft-mcp`, behind one Entra app registration for the company.
   - Device-code sign-in needs a token cache that outlives the connector's container (a mounted folder or the keyring), or sign-in happens again every start.
3. **Writes need approval:** mark sending mail and sending or accepting invites as `sensitive_tools` in the connector declaration, so they wait for the user. Drafts and reads don't.

## Steps

- [ ] Vet the chosen server: licence, maintenance, what it sends where, and whether its writes can be told apart from its reads.
- [ ] Add it as a declared connector, and as a Library catalog entry if it works well.
- [ ] Give Chief of Staff `connectors: [outlook]`, with instructions: availability before proposing times, drafts for the user to send.
- [ ] Test:
  - "What's on my calendar tomorrow?"
  - "When are Lena and Sam both free next week?"
  - "Draft an invite for the SOC 1 walkthrough": a draft, not sent.
  - "Send it": approval first.
