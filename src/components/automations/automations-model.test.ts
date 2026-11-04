import { describe, expect, it } from 'vitest'
import type { Trigger, TriggerFire, WorkflowRun } from '@/lib/bindings'
import {
  dutyText,
  currentStepNumber,
  describeSchedule,
  failedStep,
  fireView,
  isActive,
  priorityLevel,
  runStarter,
  stepLabel,
  triggersFor,
  waitingStep,
  webhookCurl,
  workflowHealth,
} from './automations-model'

const DAY = 24 * 60 * 60 * 1000

const step = (
  stepId: string,
  status: WorkflowRun['steps'][number]['status']
): WorkflowRun['steps'][number] => ({
  stepId,
  agentId: 'writer',
  template: '',
  requiresApproval: false,
  status,
  input: null,
  output: null,
  agentRunId: null,
  error: null,
  startedAt: null,
  endedAt: null,
})

const run = (overrides: Partial<WorkflowRun> = {}): WorkflowRun => ({
  id: 'r1',
  workflowId: 'w',
  status: 'completed',
  startedAt: 10 * DAY,
  endedAt: 10 * DAY + 2000,
  steps: [],
  error: null,
  source: null,
  ...overrides,
})

const trigger = (overrides: Partial<Trigger> = {}): Trigger =>
  ({
    id: 'morning',
    name: 'Morning check',
    targetType: 'workflow',
    targetId: 'w',
    ...overrides,
  }) as Trigger

describe('describeSchedule', () => {
  it('says a schedule in words, in the UI language', () => {
    expect(describeSchedule('0 9 * * 1-5', 'en')).toBe(
      'At 09:00, Monday through Friday'
    )
    expect(describeSchedule('0 9 * * 1-5', 'fr')).not.toBe(
      describeSchedule('0 9 * * 1-5', 'en')
    )
  })

  it("returns null for what doesn't parse", () => {
    expect(describeSchedule('', 'en')).toBeNull()
    expect(describeSchedule('61 * * * *', 'en')).toBeNull()
    expect(describeSchedule('nope', 'en')).toBeNull()
  })
})

describe('priorityLevel', () => {
  it('maps each stored priority to its choice, else Custom', () => {
    expect(priorityLevel(30)).toBe('high')
    expect(priorityLevel(80)).toBe('normal')
    expect(priorityLevel(100)).toBe('low')
    expect(priorityLevel(null)).toBe('normal')
    expect(priorityLevel(45)).toBe('custom')
    // A source's own default (webhooks: 60) reads as Normal
    expect(priorityLevel(60, 60)).toBe('normal')
    expect(priorityLevel(60, 80)).toBe('custom')
  })
})

describe('stepLabel', () => {
  it('turns step IDs into words', () => {
    expect(stepLabel('send-email')).toBe('Send email')
    expect(stepLabel('extract_data')).toBe('Extract data')
    expect(stepLabel('a')).toBe('A')
  })
})

describe('runStarter', () => {
  it('names every kind of start', () => {
    const triggers = [trigger()]
    expect(runStarter({ kind: 'manual' }, triggers)).toEqual({ kind: 'manual' })
    expect(runStarter({ kind: 'trigger', id: 'morning' }, triggers)).toEqual({
      kind: 'trigger',
      triggerId: 'morning',
      name: 'Morning check',
    })
    expect(runStarter({ kind: 'trigger', id: 'gone' }, triggers)).toEqual({
      kind: 'trigger',
      triggerId: 'gone',
      name: null,
    })
    expect(runStarter({ kind: 'gateway', id: 't' }, triggers)).toEqual({
      kind: 'otherNandi',
    })
    expect(runStarter({ kind: 'task', id: 't' }, triggers)).toEqual({
      kind: 'otherNandi',
    })
    expect(runStarter(null, triggers)).toEqual({ kind: 'unknown' })
  })
})

describe('fireView', () => {
  const fire = (runId: string | null) => ({ runId }) as TriggerFire
  it('opens the workflow run, or the agent run, a firing started', () => {
    expect(fireView(fire('r9'), { targetType: 'workflow' })).toEqual({
      kind: 'work-item',
      id: 'r9',
    })
    expect(fireView(fire('a9'), { targetType: 'agent' })).toEqual({
      kind: 'run-detail',
      runId: 'a9',
    })
    expect(fireView(fire(null), { targetType: 'agent' })).toBeNull()
  })
})

describe('dutyText', () => {
  const t = (key: string, o?: Record<string, unknown>) =>
    `${key} ${JSON.stringify(o)}`

  it('says who a calendar firing was for, and who took over', () => {
    expect(
      dutyText({ payload: { role: 'primary', responsible: 'Dev' } }, t)
    ).toBe('triggers.duty.onDuty {"person":"Dev"}')
    expect(
      dutyText(
        {
          payload: {
            role: 'backup',
            responsible: 'Lena',
            primary: 'Dev',
            handover_reason: "Dev's Nandi is offline",
          },
        },
        t
      )
    ).toBe(
      `triggers.duty.tookOver {"person":"Lena","primary":"Dev","reason":"Dev's Nandi is offline"}`
    )
    // Other firings, and calendars that name nobody
    expect(dutyText({ payload: { path: '/x' } }, t)).toBeNull()
    expect(
      dutyText({ payload: { role: 'primary', responsible: null } }, t)
    ).toBeNull()
  })
})

describe('workflowHealth', () => {
  it('counts the last 30 days by outcome', () => {
    const now = 40 * DAY
    const health = workflowHealth(
      [
        run({ id: 'a', startedAt: now - DAY, endedAt: now - DAY + 1000 }),
        run({
          id: 'b',
          status: 'failed',
          startedAt: now - 2 * DAY,
          endedAt: now - 2 * DAY + 3000,
        }),
        run({
          id: 'c',
          status: 'awaitingApproval',
          startedAt: now - 1000,
          endedAt: null,
        }),
        run({ id: 'old', startedAt: now - 31 * DAY }),
      ],
      now
    )
    expect(health).toEqual({
      total: 3,
      done: 1,
      failed: 1,
      waiting: 1,
      avgMs: 2000,
    })
    expect(workflowHealth([], now).avgMs).toBeNull()
  })
})

describe('run helpers', () => {
  it('find the waiting and failed steps and the step number', () => {
    const waiting = run({
      status: 'awaitingApproval',
      steps: [
        step('a', 'done'),
        step('b', 'awaitingApproval'),
        step('c', 'pending'),
      ],
    })
    expect(waitingStep(waiting)?.stepId).toBe('b')
    expect(currentStepNumber(waiting)).toBe(2)
    expect(isActive(waiting)).toBe(true)
    const failed = run({
      status: 'failed',
      steps: [step('a', 'done'), step('b', 'failed')],
    })
    expect(failedStep(failed)?.stepId).toBe('b')
    expect(failedStep(waiting)).toBeNull()
    expect(isActive(run())).toBe(false)
    expect(currentStepNumber(run({ steps: [step('a', 'done')] }))).toBe(1)
  })

  it('lists the triggers that start a workflow', () => {
    const triggers = [
      trigger(),
      trigger({ id: 'x', targetType: 'agent', targetId: 'w' }),
    ]
    expect(triggersFor('w', triggers).map(t => t.id)).toEqual(['morning'])
  })

  it('writes a webhook sample with the secret header', () => {
    expect(webhookCurl('http://127.0.0.1:1/api/triggers/webhook/h')).toContain(
      'X-Trigger-Secret: <your secret>'
    )
  })
})
