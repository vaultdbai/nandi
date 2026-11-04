/** Request fixtures for tests (the shapes `get_request` returns) */
import type { Blocker, BlockerKind, Request, RequestStep } from '@/lib/bindings'

export function step(overrides: Partial<RequestStep> = {}): RequestStep {
  return {
    id: 'step-1',
    kind: 'agent',
    stage: null,
    label: 'Portfolio Analyst',
    status: 'running',
    link: {
      agentId: 'papers',
      runId: null,
      taskId: 't1',
      workflowId: null,
      workflowRunId: null,
    },
    progress: null,
    detail: null,
    etaSeconds: null,
    attempts: [],
    children: [],
    startedAt: 1_000,
    endedAt: null,
    ...overrides,
  }
}

export function system(
  id: string,
  stage: NonNullable<RequestStep['stage']>,
  overrides: Partial<RequestStep> = {}
): RequestStep {
  return step({
    id,
    kind: 'system',
    stage,
    label: '',
    status: 'done',
    link: {
      agentId: null,
      runId: null,
      taskId: null,
      workflowId: null,
      workflowRunId: null,
    },
    ...overrides,
  })
}

export function blocker(
  kind: BlockerKind,
  overrides: Partial<Blocker> = {}
): Blocker {
  return {
    kind,
    who: 'me',
    terminal: false,
    stepId: 'step-1',
    since: 1_000,
    deadline: null,
    member: "Priya's Nandi",
    memberInstance: 'priya-laptop',
    agentId: 'papers',
    agentName: 'Portfolio Analyst',
    runId: null,
    approvalId: null,
    taskId: 't1',
    reason: null,
    queuePosition: null,
    ...overrides,
  }
}

export function request(overrides: Partial<Request> = {}): Request {
  return {
    id: 't1',
    title: 'Summarize Q3 portfolio risk',
    input: { text: 'Summarize Q3 portfolio risk', due: '2026-11-03' },
    capability: 'finance.risk',
    direction: 'incoming',
    counterpart: "Priya's Nandi",
    counterpartInstance: 'priya-laptop',
    status: 'running',
    stuck: null,
    moreWaiting: 0,
    steps: [
      system('received', 'received'),
      system('allowed', 'allowed'),
      system('routed', 'routed', {
        detail: 'Routed to Portfolio Analyst: serves it',
      }),
      step(),
      system('delivered', 'delivered', { status: 'pending', startedAt: null }),
    ],
    planSteps: 1,
    currentStep: 1,
    result: null,
    error: null,
    rating: null,
    etaSeconds: null,
    deadline: null,
    startedAt: 1_000,
    updatedAt: 2_000,
    endedAt: null,
    taskIds: ['t1', 't1-gw'],
    requesterInstance: 'priya-laptop',
    providerInstance: 'desktop-me',
    costUsd: 0.01,
    inputTokens: 100,
    outputTokens: 20,
    activity: [
      {
        at: 1_000,
        taskId: 't1',
        kind: 'state',
        fromState: null,
        toState: 'submitted',
        reason: null,
        payload: null,
      },
      {
        at: 1_100,
        taskId: 't1',
        kind: 'progress',
        fromState: null,
        toState: null,
        reason: null,
        payload: { note: 'started' },
      },
    ],
    ...overrides,
  }
}
