import { describe, expect, it } from 'vitest'
import type {
  NextAction,
  WorkItem,
  WorkState,
  WorkflowRun,
  WorkflowStepRun,
} from '@/lib/bindings'
import { nextActionMessage, runControls, stepLabel } from './work'

function item(state: WorkState, nextAction: WorkItem['nextAction']): WorkItem {
  return {
    id: 'run-1',
    kind: 'workflow',
    title: 'Onboarding',
    state,
    owner: null,
    progress: { completedSteps: 1, totalSteps: 3, percent: 33 },
    nextAction,
    workflowId: 'onboarding',
    rating: null,
    startedAt: 0,
    updatedAt: 0,
  }
}

function run(status: WorkflowRun['status']): WorkflowRun {
  return {
    id: 'run-1',
    workflowId: 'onboarding',
    status,
    startedAt: 0,
    endedAt: null,
    error: null,
    source: null,
    steps: [],
  }
}

const waitingOnPerson = {
  description: 'Tag the laptop',
  kind: 'manualStep' as const,
  deadline: null,
}

describe('runControls', () => {
  it('pauses and cancels a running run, without resume', () => {
    expect(runControls(item('active', null), run('running'))).toEqual({
      pause: true,
      resume: false,
      cancel: true,
    })
  })

  it('resumes and cancels a paused run', () => {
    expect(
      runControls(item('waiting', waitingOnPerson), run('paused'))
    ).toEqual({ pause: false, resume: true, cancel: true })
  })

  it('does not offer resume for a run waiting on a manual step', () => {
    expect(
      runControls(item('waiting', waitingOnPerson), run('waiting'))
    ).toEqual({ pause: false, resume: false, cancel: true })
  })

  it('does not offer resume before the run has loaded', () => {
    expect(runControls(item('waiting', waitingOnPerson), null).resume).toBe(
      false
    )
  })

  it('offers nothing for a finished run', () => {
    expect(runControls(item('done', null), run('completed'))).toEqual({
      pause: false,
      resume: false,
      cancel: false,
    })
  })
})

describe('stepLabel', () => {
  it('reads a step ID as words', () => {
    expect(stepLabel('draft-report')).toBe('Draft report')
    expect(stepLabel('post_review')).toBe('Post review')
  })
})

describe('nextActionMessage', () => {
  const names = (id: string) => ({ 'pr-reviewer': 'PR Reviewer' })[id] ?? id
  const step = (over: Partial<WorkflowStepRun>): WorkflowStepRun => ({
    stepId: 'post-review',
    agentId: 'pr-reviewer',
    template: '',
    requiresApproval: false,
    status: 'pending',
    input: null,
    output: null,
    agentRunId: null,
    error: null,
    startedAt: null,
    endedAt: null,
    ...over,
  })
  const withSteps = (steps: WorkflowStepRun[]): WorkflowRun => ({
    ...run('running'),
    steps,
  })
  const next = (
    kind: NextAction['kind'],
    stepId: string | null
  ): NextAction => ({
    description: 'raw ids',
    kind,
    deadline: null,
    reasonKind: null,
    target: { runId: 'run-1', stepId, agentId: null, taskId: null },
  })

  it('names the agent and the step for an approval', () => {
    expect(
      nextActionMessage(
        withSteps([step({ status: 'awaitingApproval' })]),
        next('approval', 'post-review'),
        names
      )
    ).toEqual({ key: 'approve', step: 'Post review', agent: 'PR Reviewer' })
  })

  it('says who is working on a running step', () => {
    expect(
      nextActionMessage(
        withSteps([step({ status: 'running' })]),
        next('auto', 'post-review'),
        names
      )
    ).toMatchObject({ key: 'working', agent: 'PR Reviewer' })
  })

  it("shows a manual step's own instructions", () => {
    expect(
      nextActionMessage(
        withSteps([step({ manual: true, instructions: 'Tag the laptop' })]),
        next('manualStep', 'post-review'),
        names
      )
    ).toEqual({ text: 'Tag the laptop' })
  })

  it('reads a run with no step as paused', () => {
    expect(
      nextActionMessage(withSteps([]), next('manualStep', null), names)
    ).toMatchObject({ key: 'paused' })
  })
})
