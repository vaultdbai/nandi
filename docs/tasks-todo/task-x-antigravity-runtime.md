# Task: Antigravity as an agent runtime (replacing Gemini CLI for most users)

**From:** owner, 2026-11-01: "Gemini CLI is disabled for individual users and maybe going away for all. Antigravity even has an SDK and Google seems to be using that more."
**Status:** investigated, not built. Recommendation and open questions below.
**Size:** about 3 to 4 days after a 1-day spike.

## Why

- **Gemini CLI is ending for most people.** Google stopped serving it on 2026-06-18 for Google AI Pro, Google AI Ultra and free individual accounts. Only Gemini Code Assist enterprise licences and API-key users still work ([Google's notice](https://github.com/google-gemini/gemini-cli/discussions/28017)). Its replacement is the Antigravity CLI, `agy`.
- **Nandi only knows three tools** (`claude`, `codex`, `gemini`, in `harness/detection.rs` `SPECS`). Every runtime is driven over ACP (`harness/acp.rs`). `agy` has no ACP mode: `agy --acp` and `--experimental-acp` are both rejected (checked with 1.4.12 here). So Nandi never shows it, and couldn't drive it if it did.
- **Antigravity reaches more models.** On this machine `agy models` lists Gemini 3.8 Flash, Claude Sonnet 4.6, Claude Opus 4.6 and GPT-OSS 120B.

## What exists (checked 2026-11-01)

| Option                                                                 | What it is                                                                                                                                                                                                                                                                            | For                                                                                                                                                                                                                                                       | Against                                                                                                                                                                                                                                                                                              |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A. Google's ACP server** `agy_acp_server`                            | A Google-signed ACP server, listed in the ACP registry as `antigravity-acp` (author "Google LLC"), downloaded from `dl.google.com`. Builds for Linux x86_64 and arm64, macOS arm64 and Windows ([OpenHands issue #4624](https://github.com/OpenHands/software-agent-sdk/issues/4624)) | Speaks ACP, so it fits Nandi's harness layer like `gemini --acp` does: a spec plus a small adapter                                                                                                                                                        | Not publicly documented as ACP, with no support commitment. The download is about 650 MiB (1.9 GiB unpacked), so it needs its own image variant. It crashes without IPv6 loopback unless started with `--enforce_kernel_ipv6_support=false`. Its OAuth sign-ins are interactive and tied to the host |
| **B. The official Python SDK** `google-antigravity`                    | Google's SDK for building agents on Antigravity, Apache-2.0, alpha (0.1.23, 2026-10-28). Wheels bundle the runtime for Linux glibc and musl (x86_64 and arm64), Windows and macOS ([PyPI](https://pypi.org/project/google-antigravity/))                                              | Documented and official. It streams text, thoughts and typed tool-call events. Its `ask_user` policy calls a handler before a tool runs, which maps onto Nandi's approvals. It has MCP over stdio, read-only by default, and Linux builds for the sandbox | Not ACP: Nandi needs a small shim of its own (Python, in the image) to translate. Alpha, so the API may change. No Google-account sign-in, only an API key or Vertex                                                                                                                                 |
| **C. Community ACP bridges** over `agy -p --output-format stream-json` | Several on GitHub, for example [sibbl/google-antigravity-acp](https://github.com/sibbl/google-antigravity-acp) and [jameslunardi/agy-agent-acp](https://github.com/jameslunardi/agy-agent-acp)                                                                                        | They work today, and `agy`'s stream works headless here (`init`, `step_update` with `text_delta` and usage, `result`)                                                                                                                                     | Third-party. They rely on an output format `agy --help` doesn't list, so an update can break them. They need the `agy` binary itself in the container; I found no confirmed Linux build of the CLI                                                                                                   |
| `agy agentapi` (hidden subcommand)                                     | `new-conversation`, `send-message`, `get-conversation-metadata`: a messaging interface for the Antigravity app's conversations                                                                                                                                                        |                                                                                                                                                                                                                                                           | Not ACP, and not a way to run an agent turn with tool approvals                                                                                                                                                                                                                                      |

## How each one signs in, against Nandi's rules

Company identity comes first and API keys last, and credentials never go into containers (memory: agents run only in the container).

- **Vertex / Gemini Enterprise Agent Platform** (A's `agent-platform`, B's Vertex modes): a Google Cloud project with Application Default Credentials. This is the company-identity path, like AWS SSO. Open question: can a short-lived access token from the host stand in for an ADC credentials file in the container? If not, the egress proxy has to sign the calls.
- **API key** (`GEMINI_API_KEY`): works headless for both A and B. Keep it in the keychain; the LLM proxy adds it, as for other providers.
- **Personal Google account:** interactive and tied to the host in A, and absent from B. Individuals (the people Gemini CLI just dropped) would need an API key, or the visible-browser approach. This is the weak spot.

## Recommendation

1. **Build on the SDK (B)**, with a small ACP shim of our own, `nandi-antigravity-acp`, installed in the `nandi-harness` image. It's the only option that is official, documented, has Linux builds, and maps to Nandi's approvals and MCP connectors. The shim is ours, so an SDK change is fixed in one place.
2. **Keep A as a fallback.** If Google documents `agy_acp_server`, it becomes a spec and adapter with no shim. Its size and lack of support make it a poor default today.
3. **Skip C:** third-party, an undocumented format, and no Linux CLI.
4. **Keep the Gemini CLI runtime** for enterprise and API-key users, who still have it. Label it "Gemini Code Assist / API key" so individuals aren't confused.

## Steps

- [ ] **Spike (1 day):** in a `nandi-harness` container, `pip install google-antigravity`, then run one agent turn with `GEMINI_API_KEY` from the LLM proxy. Check streaming events, an `ask_user` approval round trip, an MCP stdio connector, and the image size added.
- [ ] **The shim:** ACP on stdio to SDK `Conversation`. Map `session/prompt` to a message, the SDK's stream to `session/update` (text, thoughts, tool calls), `ask_user` to ACP `session/request_permission` (Nandi's approval cards), and MCP servers from the session to `McpStdioServer`.
- [ ] **Runtime:** a `HarnessSpec` for Antigravity (`antigravity`, "Antigravity"), detection of the shim in the image rather than `agy` on the host, an adapter alongside `gemini_cli.rs`, models from the SDK's list, and health and auth checks.
- [ ] **Sign-in:** API key through the LLM proxy first; Vertex with a company project next (needs the answer to the ADC question).
- [ ] **Docs and catalog:** Settings › Models & runtimes, `docs/developer/`, `docs/testing/` runtime entries.
- [ ] **Test:** an Antigravity agent answers, asks before running a command, uses an MCP connector, and works with Gemini and a Claude model through Antigravity.

## Open questions for the owner

- **Sign-in first:** an API key (works for everyone, today), or a company Google Cloud project (Vertex, the company-identity path)?
- **Individuals:** is an API key acceptable for people without a company Google Cloud project?
- **Fine with a Python shim in the image,** or should we wait for Google to document its ACP server?
