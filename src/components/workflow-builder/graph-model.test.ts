import { describe, expect, it } from 'vitest'
import type { Workflow } from '@/lib/bindings'
import {
  applyWorkflowEvent,
  layoutGraph,
  referencedSteps,
  workflowYamlWithStep,
} from './graph-model'
import { sampleGraph } from './test-support'

const NOW = 1_000

describe('applyWorkflowEvent', () => {
  it('resets the nodes when a new run starts a step', () => {
    const graph = applyWorkflowEvent(
      sampleGraph(),
      {
        event: 'workflow://step_started',
        payload: {
          workflowId: 'daily-report',
          runId: 'wr-2',
          stepId: 'research',
          agentId: 'research-agent',
          agentRunId: 'run-c',
          input: 'Go',
        },
      },
      NOW
    )
    expect(graph.runId).toBe('wr-2')
    expect(graph.runStatus).toBe('running')
    const [research, publish] = graph.nodes
    expect(research).toMatchObject({
      status: 'running',
      agentRunId: 'run-c',
      input: 'Go',
      startedAt: NOW,
      output: null,
    })
    expect(publish).toMatchObject({ status: 'pending', input: null })
  })

  it('tracks a step through approval and completion', () => {
    const running = sampleGraph({ runId: 'wr-1', runStatus: 'running' })
    const waiting = applyWorkflowEvent(running, {
      event: 'workflow://step_awaiting_approval',
      payload: {
        workflowId: 'daily-report',
        runId: 'wr-1',
        stepId: 'publish',
        agentId: 'publisher-agent',
        input: 'Send: x',
      },
    })
    expect(waiting.runStatus).toBe('awaitingApproval')
    expect(waiting.nodes[1]).toMatchObject({
      status: 'awaitingApproval',
      input: 'Send: x',
    })
    // The earlier step keeps its result: same run
    expect(waiting.nodes[0]?.status).toBe('done')

    const done = applyWorkflowEvent(
      waiting,
      {
        event: 'workflow://step_done',
        payload: {
          workflowId: 'daily-report',
          runId: 'wr-1',
          stepId: 'publish',
          output: 'Sent',
        },
      },
      NOW
    )
    expect(done.nodes[1]).toMatchObject({
      status: 'done',
      output: 'Sent',
      endedAt: NOW,
    })
    const finished = applyWorkflowEvent(done, {
      event: 'workflow://done',
      payload: { workflowId: 'daily-report', runId: 'wr-1' },
    })
    expect(finished.runStatus).toBe('completed')
  })

  it('fails the step and skips the steps after it', () => {
    const graph = applyWorkflowEvent(sampleGraph({ runStatus: 'running' }), {
      event: 'workflow://failed',
      payload: {
        workflowId: 'daily-report',
        runId: 'wr-1',
        stepId: 'research',
        error: 'Boom',
      },
    })
    expect(graph.runStatus).toBe('failed')
    expect(graph.nodes.map(n => n.status)).toEqual(['failed', 'skipped'])
    expect(graph.nodes[0]?.error).toBe('Boom')
  })

  it('ignores other workflows and other events', () => {
    const graph = sampleGraph()
    expect(
      applyWorkflowEvent(graph, {
        event: 'workflow://done',
        payload: { workflowId: 'other', runId: 'wr-9' },
      })
    ).toBe(graph)
    expect(
      applyWorkflowEvent(graph, { event: 'workflow://changed', payload: {} })
    ).toBe(graph)
  })
})

describe('layoutGraph', () => {
  it('lays steps out top to bottom', () => {
    const [first, second] = layoutGraph(sampleGraph())
    expect(first?.id).toBe('research')
    expect(second?.id).toBe('publish')
    expect(second?.y ?? 0).toBeGreaterThan(first?.y ?? 0)
  })
})

describe('workflowYamlWithStep', () => {
  const workflow: Pick<Workflow, 'id' | 'name' | 'steps'> = {
    id: 'daily-report',
    name: 'Daily Report',
    steps: [
      {
        id: 'research',
        agent: 'research-agent',
        input: 'Find "news"',
        requiresApproval: false,
        critical: true,
      },
      {
        id: 'publish',
        agent: 'publisher-agent',
        input: 'Send: {{ research.output }}',
        requiresApproval: true,
        continueOnFailure: true,
      },
    ],
  }

  it('replaces the step and follows a rename in later inputs', () => {
    const yaml = workflowYamlWithStep(workflow, 'research', {
      id: 'gather',
      agent: 'other-agent',
      input: 'Find "news"',
    })
    expect(yaml).toBe(
      [
        'id: "daily-report"',
        'name: "Daily Report"',
        'steps:',
        '  - id: "gather"',
        '    agent: "other-agent"',
        '    input: "Find \\"news\\""',
        '    critical: true',
        '  - id: "publish"',
        '    agent: "publisher-agent"',
        '    input: "Send: {{gather.output}}"',
        '    requires_approval: true',
        '    on_failure: continue',
        '',
      ].join('\n')
    )
  })

  it('keeps ask steps, and their references follow a renamed step', () => {
    const yaml = workflowYamlWithStep(
      {
        id: 'evidence',
        name: 'Evidence',
        steps: [
          { id: 'scope', agent: 'writer', input: 'Scope it' },
          {
            id: 'ask',
            agent: '',
            input: '',
            ask: {
              find: 'SOC 1 evidence',
              person: null,
              capability: null,
              input: 'Send evidence for {{scope.output}}',
              due: '5 days',
            },
            continueOnFailure: true,
          },
        ],
      },
      'scope',
      { id: 'plan', agent: 'writer', input: 'Scope it' }
    )
    expect(yaml).toBe(
      [
        'id: "evidence"',
        'name: "Evidence"',
        'steps:',
        '  - id: "plan"',
        '    agent: "writer"',
        '    input: "Scope it"',
        '  - id: "ask"',
        '    ask:',
        '      find: "SOC 1 evidence"',
        '      input: "Send evidence for {{plan.output}}"',
        '      due: "5 days"',
        '    on_failure: continue',
        '',
      ].join('\n')
    )
  })

  it('keeps the description and manual steps (regression: an edit dropped them)', () => {
    const yaml = workflowYamlWithStep(
      {
        id: 'onboarding',
        name: 'Onboarding',
        description: 'Gets a customer going.',
        steps: [
          { id: 'draft', agent: 'writer', input: 'Draft it' },
          {
            id: 'sign',
            agent: '',
            input: '',
            manual: true,
            instructions: 'Sign: {{draft.output}}',
            deadlineHours: 24,
          },
        ],
      },
      'draft',
      {
        id: 'write',
        agent: 'writer',
        input: 'Draft it',
        requiresApproval: true,
      }
    )
    expect(yaml).toBe(
      [
        'id: "onboarding"',
        'name: "Onboarding"',
        'description: "Gets a customer going."',
        'steps:',
        '  - id: "write"',
        '    agent: "writer"',
        '    input: "Draft it"',
        '    requires_approval: true',
        '  - id: "sign"',
        '    step_type: manual',
        '    instructions: "Sign: {{write.output}}"',
        '    deadline_hours: 24',
        '',
      ].join('\n')
    )
  })

  it('lists the steps an input refers to', () => {
    expect(
      referencedSteps('{{a.output}} and {{ b.output }} and {{a.output}}')
    ).toEqual(['a', 'b'])
  })
})
