import { describe, expect, it } from 'vitest'
import type { ChatMessage, RunDetail } from '@/lib/tauri-bindings'
import {
  agentMessage,
  approvalMessage,
  run,
  userMessage,
} from '@/test/fixtures'
import {
  detectNumericData,
  filesTouched,
  finalOutput,
  generatedFiles,
  parseNumber,
  runStats,
  timelineSteps,
} from './run-detail-model'

function toolCall(
  id: string,
  tool: string,
  args: unknown,
  overrides: Partial<Extract<ChatMessage, { kind: 'toolCall' }>> = {}
): ChatMessage {
  return {
    kind: 'toolCall',
    id,
    runId: 'r1',
    createdAt: 2,
    tool,
    args: args as never,
    status: 'done',
    result: null,
    isError: false,
    ...overrides,
  }
}

function detail(
  messages: ChatMessage[],
  overrides: Partial<RunDetail['run']> = {}
): RunDetail {
  return {
    run: run('r1', 'writer', {
      status: 'completed',
      endedAt: run('x', 'writer').startedAt + 3_000,
      inputTokens: 100,
      outputTokens: 50,
      ...overrides,
    }),
    agentName: 'Writer',
    messages,
    events: [],
  }
}

describe('timelineSteps', () => {
  it('starts, follows the messages, and ends with the run status', () => {
    const steps = timelineSteps(
      detail([
        userMessage('u1', 'Write  the\nreport'),
        toolCall('t1', 'shell', { cmd: 'ls -la' }),
        toolCall('t2', 'read_file', { path: 'x' }, { isError: true }),
        approvalMessage('a1', 'rejected'),
        agentMessage('m1', 'Done'),
      ])
    )
    expect(steps.map(s => [s.kind, s.state])).toEqual([
      ['started', 'done'],
      ['request', 'done'],
      ['tool', 'done'],
      ['tool', 'failed'],
      ['approval', 'failed'],
      ['reply', 'done'],
      ['finished', 'done'],
    ])
    expect(steps[1]?.description).toBe('Write the report')
    expect(steps[2]?.description).toBe('ls -la')
  })

  it('shows a running step while the run is active', () => {
    const steps = timelineSteps(
      detail([toolCall('t1', 'shell', {}, { status: 'running' })], {
        status: 'running',
        endedAt: null,
      })
    )
    expect(steps.at(-2)?.state).toBe('running')
    expect(steps.at(-1)).toMatchObject({ kind: 'running', state: 'running' })
  })

  it('marks failed runs red', () => {
    expect(timelineSteps(detail([], { status: 'failed' })).at(-1)?.state).toBe(
      'failed'
    )
  })

  it('gives each finished step its duration, until the next one began', () => {
    const start = run('x', 'writer').startedAt
    const steps = timelineSteps(
      detail([
        toolCall('t1', 'read_file', { path: 'a' }, { createdAt: start + 400 }),
        toolCall('t2', 'shell', {}, { createdAt: start + 2_400 }),
      ])
    )
    expect(steps.map(s => s.durationMs)).toEqual([400, 2_000, 600, undefined])
  })

  it('leaves a step still going without a duration', () => {
    const steps = timelineSteps(
      detail([toolCall('t1', 'shell', {}, { status: 'running' })], {
        status: 'running',
        endedAt: null,
      })
    )
    expect(steps.at(-2)?.durationMs).toBeUndefined()
  })
})

describe('files', () => {
  it('derives reads, writes, shell commands, and attachments with sizes', () => {
    const d = detail([
      {
        ...(userMessage('u1', 'hi') as Extract<ChatMessage, { kind: 'user' }>),
        attachments: [
          { id: 'att1', name: 'data.csv', kind: 'file', sizeBytes: 2048 },
        ],
      },
      toolCall('t1', 'write_file', { path: 'out/a.md', content: 'héllo' }),
      toolCall(
        't2',
        'filesystem__read_file',
        { path: 'in.txt' },
        { result: 'abc' }
      ),
      toolCall('t3', 'shell', { cmd: 'wc -l in.txt' }),
      toolCall('t4', 'delegate_to_agent', { path: 'ignored' }),
      toolCall('t5', 'write_file', { path: 'out/a.md', content: 'again' }),
    ])
    expect(filesTouched(d).map(f => [f.op, f.path, f.sizeBytes])).toEqual([
      ['attached', 'data.csv', 2048],
      ['write', 'out/a.md', 6],
      ['read', 'in.txt', 3],
      ['shell', 'wc -l in.txt', null],
      ['write', 'out/a.md', 5],
    ])
    // Each written path once
    expect(generatedFiles(d).map(f => f.path)).toEqual(['out/a.md'])
    expect(runStats(d)).toMatchObject({
      toolCalls: 5,
      filesWritten: 1,
      durationMs: 3_000,
    })
  })
})

describe('browser screenshots', () => {
  it('are files the run wrote, named by Nandi or by the agent', () => {
    const d = detail([
      toolCall(
        's1',
        'pilot_browser-headless__browser_take_screenshot',
        {},
        {
          result:
            "Took the screenshot.\nSaved the screenshot as browser/screenshot-1.png in the agent's files.",
        }
      ),
      toolCall('s2', 'pilot_browser__browser_take_screenshot', {
        filename: 'browser/home.png',
      }),
    ])
    expect(generatedFiles(d).map(f => f.path)).toEqual([
      'browser/screenshot-1.png',
      'browser/home.png',
    ])
  })
})

describe('finalOutput', () => {
  it('is the last agent message, else the run summary', () => {
    expect(
      finalOutput(
        detail([agentMessage('m1', 'one'), agentMessage('m2', 'two')])
      )
    ).toBe('two')
    expect(finalOutput(detail([], { summary: 'sum' }))).toBe('sum')
  })
})

describe('detectNumericData', () => {
  it('parses numbers as written', () => {
    expect(parseNumber('$1,234.5')).toBe(1234.5)
    expect(parseNumber('**12%**')).toBe(12)
    expect(parseNumber('+3')).toBe(3)
    expect(parseNumber('-0.4')).toBe(-0.4)
    expect(parseNumber('12 apples')).toBeNull()
    expect(parseNumber('')).toBeNull()
  })

  it('reads a fenced JSON object of numbers', () => {
    const data = detectNumericData(
      'Results:\n```json\n{"positions": 127, "risk": 3.2, "name": "x"}\n```'
    )
    expect(data?.points.map(p => [p.label, p.value])).toEqual([
      ['positions', 127],
      ['risk', 3.2],
    ])
  })

  it('reads a JSON series', () => {
    const data = detectNumericData('{"revenue": [1, 2, 3]}')
    expect(data?.title).toBe('revenue')
    expect(data?.points).toHaveLength(3)
  })

  it('reads the first numeric column of a markdown table', () => {
    const data = detectNumericData(
      [
        'Summary below.',
        '',
        '| Month | Note | Revenue |',
        '| --- | --- | ---: |',
        '| Jan | ok | $1,200 |',
        '| Feb | up | $1,450 |',
        '',
        'Done.',
      ].join('\n')
    )
    expect(data?.title).toBe('Revenue')
    expect(data?.points.map(p => [p.label, p.value, p.display])).toEqual([
      ['Jan', 1200, '$1,200'],
      ['Feb', 1450, '$1,450'],
    ])
  })

  it('ignores prose numbers and lone values', () => {
    expect(
      detectNumericData('Processed 1,248 records in 3 minutes.')
    ).toBeNull()
    expect(detectNumericData('{"count": 3}')).toBeNull()
    expect(detectNumericData(null)).toBeNull()
  })
})
