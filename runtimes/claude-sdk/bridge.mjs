// Nandi's bridge to Claude Code through the Claude Agent SDK
// (@anthropic-ai/claude-agent-sdk), for Nandi's own model calls.
//
// It drives the user's `claude` executable for one plain completion, so
// the call signs in the way Claude Code does (a company Claude login, or
// Bedrock/Vertex/Foundry through Claude Code's settings or the env Nandi
// passes). Nothing else of Claude Code is used: no tools, no MCP servers,
// no hooks, no settings or project files, one turn. Agents' code never
// runs here; it runs in the sandbox container.
//
// Protocol: JSON lines on stdio (see src-tauri/src/services/harness/claude_sdk.rs).
//   stdin, one line: { claudePath, cwd, model?, systemPrompt, prompt }
//   stdout: { kind: "ready" } once the SDK is loaded, then
//     { kind: "messageDelta", text }…, ending with
//     { kind: "turnFinished", stopReason, usage } or { kind: "error", message }

import process from 'node:process'
import { createInterface } from 'node:readline'
import { query } from '@anthropic-ai/claude-agent-sdk'

const out = message => process.stdout.write(`${JSON.stringify(message)}\n`)
const fail = message => {
  out({ kind: 'error', message: String(message) })
  process.exit(1)
}

const usageOf = result => {
  const usage = result?.usage ?? {}
  const cached =
    (usage.cache_read_input_tokens ?? 0) +
    (usage.cache_creation_input_tokens ?? 0)
  return {
    inputTokens: (usage.input_tokens ?? 0) + cached,
    outputTokens: usage.output_tokens ?? 0,
    cachedInputTokens: usage.cache_read_input_tokens ?? 0,
    costUsd: null,
  }
}

async function complete(config) {
  const q = query({
    prompt: config.prompt ?? '',
    options: {
      pathToClaudeCodeExecutable: config.claudePath || undefined,
      cwd: config.cwd || process.cwd(),
      model: config.model || undefined,
      systemPrompt: config.systemPrompt ?? '',
      tools: [],
      mcpServers: {},
      strictMcpConfig: true,
      settingSources: [],
      maxTurns: 1,
      persistSession: false,
      includePartialMessages: true,
      env: { ...process.env, CLAUDE_AGENT_SDK_CLIENT_APP: 'nandi' },
      stderr: line => process.stderr.write(line),
    },
  })
  let streamed = false
  for await (const message of q) {
    if (message.type === 'stream_event') {
      const delta = message.event?.delta
      if (delta?.type === 'text_delta' && delta.text) {
        streamed = true
        out({ kind: 'messageDelta', text: delta.text })
      }
    } else if (message.type === 'assistant' && !streamed) {
      for (const block of message.message?.content ?? []) {
        if (block.type === 'text' && block.text)
          out({ kind: 'messageDelta', text: block.text })
      }
    } else if (message.type === 'result') {
      if (message.is_error) {
        const reason =
          message.result ||
          (message.errors ?? []).join('\n') ||
          message.subtype ||
          'error'
        out({ kind: 'error', message: reason })
      } else {
        out({
          kind: 'turnFinished',
          stopReason: message.stop_reason ?? 'end_turn',
          usage: usageOf(message),
        })
      }
      return
    }
  }
  out({ kind: 'error', message: 'Claude Code ended without an answer' })
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity })[
  Symbol.asyncIterator
]()
const first = await lines.next()
if (first.done) process.exit(0)
let config
try {
  config = JSON.parse(first.value)
} catch {
  fail('The first line must be the request (JSON)')
}

out({ kind: 'ready' })
try {
  await complete(config)
  process.exit(0)
} catch (error) {
  fail(error?.message ?? error)
}
