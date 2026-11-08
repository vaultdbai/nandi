import { describe, expect, it } from 'vitest'
import type { ChatMessage, RunPlan, RunPlanStep } from '@/lib/bindings'
import {
  fileName,
  hiddenPlanCalls,
  isPlanCard,
  isScreenshot,
  missesScreenshot,
  planProgress,
} from './run-plan-model'

const call = (
  id: string,
  tool: string,
  over: Partial<Extract<ChatMessage, { kind: 'toolCall' }>> = {}
): ChatMessage => ({
  kind: 'toolCall',
  id,
  runId: 'r1',
  createdAt: 0,
  tool,
  args: {},
  status: 'done',
  result: '',
  isError: false,
  ...over,
})

const step = (over: Partial<RunPlanStep> = {}): RunPlanStep => ({
  title: 'Open the site',
  kind: 'navigate',
  evidence: 'none',
  note: null,
  status: 'pending',
  result: null,
  files: [],
  ...over,
})

describe('run plan model', () => {
  it('shows one card per run, for its last plan', () => {
    const messages = [
      call('set-1', 'plan_set'),
      call('update-1', 'plan_update'),
      call('read', 'read_file'),
      call('set-2', 'plan_set'),
      call('other-run', 'plan_set', { runId: 'r2' }),
      call('refused-update', 'plan_update', { isError: true }),
      call('refused-set', 'plan_set', { isError: true }),
    ]
    const hidden = hiddenPlanCalls(messages)
    expect([...hidden].sort()).toEqual(['set-1', 'update-1'])
    expect(messages.filter(m => isPlanCard(m)).map(m => m.id)).toEqual([
      'set-1',
      'set-2',
      'other-run',
    ])
  })

  it('needs a run for a card', () => {
    expect(isPlanCard(call('x', 'plan_set', { runId: null }))).toBe(false)
    expect(isPlanCard(call('x', 'plan_update'))).toBe(false)
  })

  it('counts done and skipped steps as closed', () => {
    const plan: RunPlan = {
      runId: 'r1',
      revision: 2,
      updatedAt: 0,
      steps: [
        step({ status: 'done' }),
        step({ status: 'skipped' }),
        step({ status: 'doing' }),
        step({ status: 'failed' }),
      ],
    }
    expect(planProgress(plan)).toEqual({ closed: 2, total: 4 })
  })

  it('knows screenshots and missing evidence', () => {
    expect(isScreenshot('browser/r1/1-1.png')).toBe(true)
    expect(isScreenshot('browser/r1/1-1.JPEG')).toBe(true)
    expect(isScreenshot('out/report.md')).toBe(false)
    expect(fileName('browser/r1/1-1.png')).toBe('1-1.png')
    expect(missesScreenshot(step({ evidence: 'screenshot' }))).toBe(false)
    expect(
      missesScreenshot(step({ evidence: 'screenshot', status: 'done' }))
    ).toBe(true)
    expect(
      missesScreenshot(
        step({ evidence: 'screenshot', status: 'done', files: ['a.png'] })
      )
    ).toBe(false)
  })
})
