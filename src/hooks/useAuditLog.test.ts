import { describe, it, expect, vi } from 'vitest'
import { commands } from '@/lib/tauri-bindings'
import type { AuditEntry } from '@/lib/tauri-bindings'
import {
  entriesToJsonl,
  fetchAuditLog,
  matchesAuditSearch,
  REDACTED,
  redactEntry,
  redactText,
  redactValue,
  SECRET_KEY_PATTERN,
} from './useAuditLog'

const ok = <T>(data: T) => ({ status: 'ok' as const, data })

function entry(overrides: Partial<AuditEntry> = {}): AuditEntry {
  return {
    timestamp: 1_000,
    kind: 'llm',
    agentId: 'writer',
    tool: 'claude:claude-sonnet-5',
    argsRedacted: { inputTokens: 10 },
    outcome: 'ok',
    durationMs: 120,
    ...overrides,
  }
}

describe('secret redaction', () => {
  it('matches password|token|secret|key field names', () => {
    for (const key of [
      'password',
      'apiToken',
      'client_secret',
      'apiKey',
      'KEY',
    ])
      expect(SECRET_KEY_PATTERN.test(key), key).toBe(true)
    for (const key of ['target', 'model', 'status', 'agent_id'])
      expect(SECRET_KEY_PATTERN.test(key), key).toBe(false)
  })

  it('redacts secret-named values but keeps counts and flags', () => {
    expect(
      redactValue({
        password: 'hunter2',
        nested: { apiKey: 'abc', list: [{ token: 'xyz' }] },
        accessKey: ['a', 'b'],
        inputTokens: 120,
        hasSecret: true,
        token: null,
        target: 'GET https://example.com',
      })
    ).toEqual({
      password: REDACTED,
      nested: { apiKey: REDACTED, list: [{ token: REDACTED }] },
      accessKey: REDACTED,
      inputTokens: 120,
      hasSecret: true,
      token: null,
      target: 'GET https://example.com',
    })
  })

  it('redacts secrets inside free text', () => {
    expect(redactText('GET https://x.test/cb?token=abc123&page=2')).toBe(
      `GET https://x.test/cb?token=${REDACTED}&page=2`
    )
    expect(redactText('Authorization: Bearer eyJhbGci.payload')).toBe(
      `Authorization: Bearer ${REDACTED}`
    )
    expect(redactText('password = hunter2')).toBe(`password = ${REDACTED}`)
    expect(redactText('api_key: "s3cr3t"')).toBe(`api_key: "${REDACTED}"`)
    expect(redactText('401 for sk-ant-0123456789abcdefghij')).toBe(
      `401 for ${REDACTED}`
    )
    expect(redactText('used AKIAABCDEFGHIJKLMNOP today')).toBe(
      `used ${REDACTED} today`
    )
    expect(redactText('nothing to see')).toBe('nothing to see')
  })

  it('redacts every text field of an entry', () => {
    const redacted = redactEntry(
      entry({
        tool: 'fetch?token=abc',
        outcome: 'error: secret=xyz',
        argsRedacted: { error: 'Bearer abc', secret: 'x' },
      })
    )
    expect(redacted.tool).toBe(`fetch?token=${REDACTED}`)
    expect(redacted.outcome).toBe(`error: secret=${REDACTED}`)
    expect(redacted.argsRedacted).toEqual({
      error: `Bearer ${REDACTED}`,
      secret: REDACTED,
    })
  })
})

describe('matchesAuditSearch', () => {
  const e = entry({ argsRedacted: { target: 'GET https://example.com' } })

  it('matches any column, case-insensitively', () => {
    expect(matchesAuditSearch(e, '')).toBe(true)
    expect(matchesAuditSearch(e, 'WRITER')).toBe(true)
    expect(matchesAuditSearch(e, 'sonnet')).toBe(true)
    expect(matchesAuditSearch(e, 'example.com')).toBe(true)
    expect(matchesAuditSearch(e, 'nope')).toBe(false)
  })

  it('narrows to one column', () => {
    expect(matchesAuditSearch(e, 'example', 'details')).toBe(true)
    expect(matchesAuditSearch(e, 'example', 'agent')).toBe(false)
    expect(matchesAuditSearch(e, 'ok', 'outcome')).toBe(true)
  })
})

describe('raw log download', () => {
  it('is JSON lines, oldest first', () => {
    const jsonl = entriesToJsonl([
      entry({ timestamp: 2 }),
      entry({ timestamp: 1 }),
    ])
    const lines = jsonl.trimEnd().split('\n')
    expect(lines.map(line => JSON.parse(line).timestamp)).toEqual([1, 2])
    expect(entriesToJsonl([])).toBe('')
  })

  it('pages through the whole log, redacted', async () => {
    const all = Array.from({ length: 700 }, (_, i) =>
      entry({ timestamp: i, argsRedacted: { password: 'p' } })
    )
    commands.listAuditLog = vi.fn(
      async (_kind: string, limit: number, offset: number) =>
        ok(all.slice(offset, offset + limit))
    )

    const entries = await fetchAuditLog('egress')

    expect(entries).toHaveLength(700)
    const passwords = entries.map(
      e => (e.argsRedacted as Record<string, unknown>)['password']
    )
    expect(passwords.every(p => p === REDACTED)).toBe(true)
    expect(vi.mocked(commands.listAuditLog).mock.calls).toEqual([
      ['egress', 500, 0],
      ['egress', 500, 500],
    ])
  })
})
