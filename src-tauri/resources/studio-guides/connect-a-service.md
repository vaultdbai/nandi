---
id: connect-a-service
title: Connect a service
when: Folders, GitHub, AWS, S3 backup, web search, MCP servers.
---

Agents reach outside services through **connectors**. A connector is installed once, set up in Library › Tools › the connector › Setup, and started there. An agent uses it only when its skill.md lists the connector under `connectors`; its tools appear as `<connector>__<tool>`. Check what's installed with `nandi_list_connectors`, and install one with `nandi_save_artifact` (kind `connector`, then the connector's `id`). The user approves the save.

## Built-in connectors

| Connector                 | What it gives                                                                                                       | Setup the user does                                                                                                                                                                   | Waits for approval                                    |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Filesystem (`filesystem`) | Read, and if allowed write, files in folders the user shares                                                        | Add a folder with a short name, read only or read and write                                                                                                                           | writing                                               |
| Web search (web_search)   | Search the web and read pages, only on the sites the agent may reach (its `network.allow`)                          | Nothing: no account or key. Add the sites the agent may search to its Internet access (`network: allow:`); `*` means any site                                                         | —                                                     |
| GitHub (`github`)         | Read pull requests and files, post a review with inline comments                                                    | Sign in with GitHub (Setup or Settings › Account); or a pasted fine-grained token with Contents read and Pull requests read/write; or the GitHub CLI (gh auth login)                  | posting a review                                      |
| AWS (`aws`)               | CloudWatch alarms and Logs (Insights queries, recent events), AWS Batch jobs, restarting a failed job               | The AWS CLI installed, and in Setup an AWS **profile** (keys, SSO or credential process) and **region** that can read CloudWatch, Logs and Batch                                      | restarting a job                                      |
| S3 Backup (`s3_backup`)   | Back up assigned folders to the user's S3 bucket as Glacier Deep Archive; create the bucket and an upload-only user | The AWS CLI; in Setup the setup **profile**, **region**, **S3 bucket** and **Folders to back up** (saving adds the backup triggers). The Personal Backup pack's Backup Keeper runs it | opening the sign-up page, creating the bucket or user |

The GitHub, AWS and S3 Backup calls run on the user's computer, never in an agent's container, so their credentials stay there. If a call says the profile isn't allowed or the sign-in expired, pass on what it says to do (for example `aws sso login`). Don't work around it.

Folders: give an agent only the ones it needs, by short name, as the build-an-agent guide shows.

## MCP servers

Any other service (Outlook, Slack, Jira, a database…) can come from an **MCP server**, saved as a connector of kind `mcp`:

```yaml
- id: outlook # short, lower-case; tools appear as outlook__<tool>
  kind: mcp
  config:
    name: Outlook # what Library shows
    command: uvx # a local server: command and args (npx, uvx, a binary)…
    args: [microsoft-mcp]
    # url: https://…/mcp      # …or a remote server over HTTP
    env: { MS_TENANT: common } # non-secret settings only
    network: [graph.microsoft.com, login.microsoftonline.com] # hosts it may reach
    safe_tools: [list_emails, get_calendar, check_availability] # run without asking
  secret_refs:
    MS_CLIENT_SECRET: nandi.connector.outlook.client_secret # env var <- keychain entry
```

- **Secrets never go in the file or the chat.** The secret_refs section maps an environment variable to a keychain entry named `nandi.connector.<id>.<name>`. The user types the value in Setup. For a remote server, a secret named bearer_token is sent as its bearer token.
- **Every tool not listed in safe_tools waits for the user's approval.** List only read-only tools (list, get, search, read).
- **Network:** a local server runs in its own locked-down container. It can reach the package registries and only the hosts listed under network. Take them from the server's docs.
- **Desktop apps:** a server that drives an app on this computer (classic Outlook through COM) can't run in a container. It needs a policy letting it run on the host for that agent (approvals-and-policies guide).

**From a pasted config.** READMEs usually give `{"mcpServers": {"<name>": {"command": …, "args": […], "env": {…}}}}`. Convert each server:

- the key becomes the id and the name
- command and args stay as they are, or url for a remote server
- each env value that is a key, token or password becomes a secret_refs entry, which the user enters in Setup (never copy the value)
- plain settings stay in env
- an `Authorization: Bearer …` header becomes the bearer_token secret

**Signing in:**

- **Keys and tokens** work today, through Setup.
- **Browser sign-in:** a remote server that only signs in with OAuth in the browser isn't supported yet. Use its API key or personal token if it has one, and otherwise say so.
- **A local server that signs in by itself** (a device code or a browser) can't open the browser from its container, and loses its login when the container stops. Prefer a key, a client ID and secret, or running on the host.

## Steps

1. Find out which service and which server. If the user named only a service, suggest a well-known server: the vendor's own first, then a widely used community one. Say what it needs.
2. Check `nandi_list_connectors`. A built-in may already do it, or it may already be installed.
3. Draft the entry and tell the user in plain words:
   - what will run
   - which hosts it can reach
   - which tools run without asking
   - which secrets or sign-ins they'll set up
4. Save it with `nandi_save_artifact` (kind `connector`, and for MCP a connector object with kind `mcp`, its config and secret_refs). The user approves.
5. Add the connector to the agent that should use it (`connectors` in its skill.md). Fork a marketplace agent first, because it's read-only.
6. Tell the user what's left: enter the secrets and click **Start** in Library › Tools, plus any sign-in or policy.

## Don't

- Don't put secret values in the config, the env or the chat. Don't ask the user to paste them into Ask.
- Don't mark write or send tools as safe.
- Don't add a server the user didn't ask for.
