---
id: runtimes-and-sign-in
title: Runtimes and sign-in
when: Agent runtimes, CLIs, sign-in, keys, free tokens.
---

## Runtimes

An agent's `runtime` says what drives it.

- **Native** (the default): Nandi itself runs the model and calls the agent's tools. Approvals are enforced. Choose this unless the user asks for a coding CLI.
- **CLI runtimes** (Claude Code, Codex, Gemini CLI): that CLI drives the agent. Set the CLI's own model in `runtime_config.model`, not in `model`. Its tool calls follow the agent's `allowed-tools` and `requires_approval`, as for native agents. It always runs in a container, and needs an API key there.

Agent code runs only in the sandbox container, never on the user's computer. That holds for every runtime: a CLI runtime runs the CLI inside the container. Only connectors and pilots reach outside, through Nandi. No host config folder is mounted into the container.

## Sign-in order

Company identity first, keys last.

1. **The team's choice**: a Bedrock setup in the repo (`.nandi/llm.yaml`) or on the agent. AWS SSO, SAML or assume-role on the host becomes short-lived keys for one session.
2. **Claude Code's own sign-in**: a Claude Team or Enterprise login on this computer. Nandi's own calls to Claude models use it too.
3. **API keys** pasted in Settings › Models (Anthropic, OpenAI, DeepSeek). Keys stay in the OS keychain and never enter a container.
4. **Ollama** on this computer needs no key.

## Free tokens

Nandi ships with free tokens on DeepSeek Flash: 100,000 per computer, so a new user can try it before setting anything up. They switch off as soon as the user sets up a model of their own (a key, a sign-in, or Ollama). While tokens are left, the user can turn them back on in Settings › Models. When they run out, the user needs their own model.

## Steps

1. Call `nandi_list_models` to see what this Nandi can call now.
2. For a corporate user, suggest their company sign-in before a key.
3. If nothing works, say exactly where to fix it: Settings › Models, or Settings › Runtimes for CLI runtimes. For AWS, Settings › Cloud credentials signs in (Log in) and sets up new profiles: **Add SSO profile** signs in to the company's access portal and picks the account and role, **Add access keys** saves keys. The user doesn't need a terminal.
4. Before showing an agent with a CLI runtime, run `nandi_check_artifact`.
