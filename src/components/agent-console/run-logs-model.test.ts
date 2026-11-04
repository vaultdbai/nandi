import { describe, expect, it } from 'vitest'
import type { JsonValue, RunDetail, RunEventRecord } from '@/lib/bindings'
import {
  agentMessage,
  approvalMessage,
  run,
  userMessage,
} from '@/test/fixtures'
import {
  filterLogs,
  inputHeadline,
  isOutwardTool,
  logEntry,
  ranOnCliRuntime,
  rerunNeedsConfirm,
  runtimeDecisions,
  stopNeedsConfirm,
  type LogEntry,
} from './run-logs-model'

const event = (kind: string, payload: JsonValue, id = 1): RunEventRecord => ({
  id,
  runId: 'r1',
  kind,
  payload,
  createdAt: 1000,
})

const toolMessage = (tool: string, args: JsonValue = { path: 'a.txt' }) => ({
  kind: 'toolCall',
  tool,
  args,
})

describe('logEntry', () => {
  // Every row of the event → line table
  const rows: [RunEventRecord, Pick<LogEntry, 'kind' | 'line' | 'values'>][] = [
    [
      event('agent://run', { run: { status: 'running' } }),
      { kind: 'other', line: 'run', values: { status: 'running' } },
    ],
    [
      event('agent://run', { run: { status: 'failed' } }),
      { kind: 'error', line: 'run', values: { status: 'failed' } },
    ],
    [
      event('agent://message', {
        message: { kind: 'user', content: 'Hi\nthere' },
      }),
      { kind: 'other', line: 'request', values: { text: 'Hi there' } },
    ],
    [
      event('agent://message', { message: { kind: 'agent', content: 'Done' } }),
      { kind: 'model', line: 'reply', values: { text: 'Done' } },
    ],
    [
      event('agent://message', { message: { kind: 'error', content: 'Boom' } }),
      { kind: 'error', line: 'error', values: { text: 'Boom' } },
    ],
    [
      event('agent://message', { message: toolMessage('read_file') }),
      {
        kind: 'tool',
        line: 'toolCall',
        values: { tool: 'read_file', detail: 'a.txt' },
      },
    ],
    [
      event('agent://message', {
        message: { ...toolMessage('shell', { cmd: 'ls' }), kind: 'approval' },
      }),
      {
        kind: 'approval',
        line: 'approvalRequest',
        values: { tool: 'shell', detail: 'ls' },
      },
    ],
    [
      event('agent://message', { message: { kind: 'other' } }),
      { kind: 'other', line: 'message', values: {} },
    ],
    [event('agent://token', {}), { kind: 'model', line: 'token', values: {} }],
    [
      event('agent://tool_call', {
        message: toolMessage('write_file', { path: 'out.md' }),
      }),
      {
        kind: 'tool',
        line: 'toolCall',
        values: { tool: 'write_file', detail: 'out.md' },
      },
    ],
    [
      event('agent://tool_result', { result: 'ok', isError: false }),
      { kind: 'tool', line: 'toolResult', values: { text: 'ok' } },
    ],
    [
      event('agent://tool_result', { result: 'denied', isError: true }),
      { kind: 'error', line: 'toolFailed', values: { text: 'denied' } },
    ],
    [
      event('agent://approval_request', {
        message: toolMessage('shell', { command: 'rm x' }),
      }),
      {
        kind: 'approval',
        line: 'approvalRequest',
        values: { tool: 'shell', detail: 'rm x' },
      },
    ],
    [
      event('agent://approval_resolved', { status: 'approved' }),
      {
        kind: 'approval',
        line: 'approvalResolved',
        values: { status: 'approved' },
      },
    ],
    [
      event('agent://turn_done', { status: 'completed' }),
      { kind: 'other', line: 'turnDone', values: { status: 'completed' } },
    ],
    [
      event('agent://turn_done', { status: 'failed' }),
      { kind: 'error', line: 'turnDone', values: { status: 'failed' } },
    ],
    [
      event('agent://turn_aborted', { reason: 'Too many steps' }),
      {
        kind: 'error',
        line: 'turnAborted',
        values: { reason: 'Too many steps' },
      },
    ],
    [
      event('agent://run_compacted', {}),
      { kind: 'other', line: 'compacted', values: {} },
    ],
    [
      event('agent://tools_loaded', { tools: ['a', 'b'] }),
      { kind: 'other', line: 'toolsLoaded', values: { count: '2' } },
    ],
    [
      event('harness://permission_request', {
        title: 'Edit a.ts',
        decision: 'allow_once',
      }),
      {
        kind: 'approval',
        line: 'permission',
        values: { title: 'Edit a.ts', decision: 'allow_once' },
      },
    ],
    [
      event('agent://something_new', { x: 1 }),
      {
        kind: 'other',
        line: 'unknown',
        values: { kind: 'agent://something_new' },
      },
    ],
  ]

  it.each(rows)('%# maps %o', (raw, expected) => {
    expect(logEntry(raw)).toMatchObject({ ...expected, id: '1', at: 1000, raw })
  })

  it('reads a payload that is not an object', () => {
    expect(logEntry(event('agent://tool_result', null)).line).toBe('toolResult')
  })
})

describe('filterLogs', () => {
  const entries = [
    logEntry(event('agent://run', { run: { status: 'running' } }, 1)),
    logEntry(
      event('agent://tool_result', { result: 'denied', isError: true }, 2)
    ),
  ]
  const lineOf = (entry: LogEntry) =>
    `${entry.line} ${Object.values(entry.values).join(' ')}`

  it('narrows by kind and by text, in the line or the raw event', () => {
    expect(filterLogs(entries, { kind: null, text: '' }, lineOf)).toHaveLength(
      2
    )
    expect(
      filterLogs(entries, { kind: 'error', text: '' }, lineOf).map(e => e.id)
    ).toEqual(['2'])
    expect(
      filterLogs(entries, { kind: null, text: 'RUNNING' }, lineOf).map(
        e => e.id
      )
    ).toEqual(['1'])
    // Only in the raw payload
    expect(
      filterLogs(entries, { kind: null, text: 'iserror' }, lineOf).map(
        e => e.id
      )
    ).toEqual(['2'])
  })
})

const detail = (overrides: Partial<RunDetail> = {}): RunDetail => ({
  run: run('r1', 'writer', { status: 'completed' }),
  agentName: 'Writer',
  messages: [userMessage('u1', 'Hi', 'r1'), agentMessage('m1', 'Done', 'r1')],
  events: [],
  ...overrides,
})

const call = (id: string, tool: string) => ({
  kind: 'toolCall' as const,
  id,
  runId: 'r1',
  createdAt: 0,
  tool,
  args: {},
  status: 'done' as const,
  result: '',
  isError: false,
})

describe('CLI runtimes', () => {
  it('spots a run from its harness tool calls or permission events', () => {
    expect(ranOnCliRuntime(detail())).toBe(false)
    expect(
      ranOnCliRuntime(detail({ messages: [call('harness-s1-t1', 'Read a')] }))
    ).toBe(true)
    const permission = event('harness://permission_request', {
      title: 'Run pnpm test',
      decision: 'reject_once',
    })
    const withPermission = detail({ events: [permission] })
    expect(ranOnCliRuntime(withPermission)).toBe(true)
    expect(runtimeDecisions(withPermission)).toEqual([
      { id: '1', at: 1000, title: 'Run pnpm test', allowed: false },
    ])
  })
})

describe('side effects', () => {
  it.each([
    ['write_file', true],
    ['filesystem__write_file', true],
    ['send_email', true],
    ['sendEmail', true],
    ['shell', true],
    ['delegate_to_agent', true],
    ['execute', true],
    ['read_file', false],
    ['list_dir', false],
    ['nandi_list_runs', false],
    ['overview', false],
  ])('%s is outward: %s', (tool, outward) => {
    expect(isOutwardTool(tool)).toBe(outward)
  })

  it('asks before re-running a run that did outward work', () => {
    expect(rerunNeedsConfirm(detail())).toBe(false)
    expect(
      rerunNeedsConfirm(detail({ messages: [call('t1', 'read_file')] }))
    ).toBe(false)
    expect(
      rerunNeedsConfirm(detail({ messages: [call('t1', 'write_file')] }))
    ).toBe(true)
  })

  it('asks before stopping with outward work or a waiting approval', () => {
    expect(stopNeedsConfirm(detail())).toBe(false)
    expect(
      stopNeedsConfirm(
        detail({ messages: [approvalMessage('a1', 'pending', 'r1')] })
      )
    ).toBe(true)
  })
})

describe('inputHeadline', () => {
  it('takes the first line, cut at 60 characters', () => {
    expect(inputHeadline('  Summarize this\nand more')).toBe('Summarize this')
    expect(inputHeadline('x'.repeat(80))).toHaveLength(60)
    expect(inputHeadline('')).toBe('')
  })
})
