# Runtime Routing

An agent's `runtime:` decides what runs its turns.

```yaml
---
name: Reviewer
runtime: claude-code # native (default) | claude-code | codex | gemini-cli | custom:<id>
runtime_config:
  model: us.anthropic.claude-sonnet-5 # the harness's model ID; omitted: its default
  provider: bedrock # optional: anthropic | bedrock | openai | …; omitted: the CLI's own
  placement: container # the only place agents run; a retired `host` is ignored and rewritten on save
  gateway: litellm # third-party Bedrock models through the LiteLLM gateway
  aws_profile: work # optional: else AWS_PROFILE / the CLI's settings
  aws_region: us-west-2
  fallback: native # optional: run natively if the harness can't start
---
```

Agents without `runtime:` are native, and their skill.md is unchanged.
Unknown `runtime_config` keys are kept on save.

## Flow

```text
execute_run ─▶ harness_run (orchestrator.rs)
  native / Studio / no harness services ─▶ the native loop, unchanged
  else resolve_runtime (runtime_router.rs)
    gathers: harness image present? CLI detected? default harness model,
             workdir, credentials for the provider, the gateway (started
             when the agent needs it and it isn't serving that model)
    plan (pure) ─▶ Native | Harness(plan) | Unavailable(reason)
  Unavailable ─▶ the run fails with the reason
  Harness     ─▶ execute_harness_run: start the session, prompt once,
                 stream updates into the run, end it
```

- **One prompt per run.** The run's input goes in as one ACP prompt, with
  the agent's system prompt in front of it in an `<instructions>` block,
  since ACP has no stable system-prompt field. The harness runs its own
  loop, so Nandi's middlewares (turn limits, compaction, verify-on-stop) do
  not apply.
- **Approvals.** A harness run doesn't suspend: the CLI waits on its ACP
  permission request instead. Kinds in `requires_approval` (mapped through
  `TOOL_KINDS`; default `shell` and `write_file`) go to the Inbox as approval
  rows marked `harness:` (`harness/approvals.rs`); decisions reach the
  waiting call through `approvals::decide`, never the orchestrator, which
  would restart the run natively. Nobody deciding within an hour rejects the
  call; a run that ends stops waiting. After a restart nothing waits, so
  deciding such an approval cancels its run.
- **Steps.** `event_translation::TurnTranslator` writes text, thinking, and
  tool calls as native steps (`agent://token`, `agent://tool_call`,
  `agent://tool_result`), so chat, history, and run details render them.
- **Permissions.** ACP permission requests are answered automatically. The
  agent's `allowed-tools` map to ACP tool kinds (`shell` → `execute`,
  `read_file` → `read`/`search`, `write_file` → `edit`/`delete`/`move`),
  and anything else is rejected. No `allowed-tools` means everything is
  allowed, because the container is the sandbox.
- **Evidence.** The run's usage and `runs.runtime` are recorded
  (`harness::evidence`). Capability evidence aggregates runs and tasks, so
  harness runs count with no change.
- **Cancel / kill.** Cancelling the run aborts its task, and the session
  row is closed by a drop guard. Killing the session (`kill_harness_session`)
  stops the CLI and its container, and the run fails.
- **Fallback is opt-in.** A harness that can't start fails the run, unless
  `fallback: native`. Falling back silently would also switch the provider
  (and where the data goes) away from what the agent chose.

## Credentials by provider

Credentials come from the user's own setup and reach only the CLI's
process (see `harness-container.md`):

| Provider           | From                                                                   |
| ------------------ | ---------------------------------------------------------------------- |
| bedrock            | the AWS identity (`aws_auth::bedrock_credentials`): exported STS keys  |
| anthropic (Claude) | Claude Code's own sign-in first; Nandi's Anthropic key only without it |
| openai (Codex)     | Nandi's OpenAI key in the keyring, if any                              |
| anything else      | nothing from Nandi: the CLI's own config and environment               |

The provider is `runtime_config.provider`, else `bedrock` with a gateway,
else what detection found in the CLI's own config. That is how a machine
where Claude Code already runs on Bedrock needs no Nandi setup.
