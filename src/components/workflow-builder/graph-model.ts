/**
 * Pure helpers for the workflow builder: query keys, live status from
 * `workflow://*` events, the dagre layout, and YAML for edited steps.
 */

import dagre from '@dagrejs/dagre'
import type { NandiEvent } from '@/lib/client'
import type {
  GraphNode,
  Workflow,
  WorkflowGraph,
  WorkflowStep,
} from '@/lib/bindings'

export type StepStatus =
  'pending' | 'running' | 'awaitingApproval' | 'done' | 'failed' | 'skipped'

const STATUSES: readonly StepStatus[] = [
  'pending',
  'running',
  'awaitingApproval',
  'done',
  'failed',
  'skipped',
]

export function stepStatus(node: GraphNode): StepStatus {
  return (STATUSES as readonly string[]).includes(node.status)
    ? (node.status as StepStatus)
    : 'pending'
}

/**
 * Under `['workflow', id]`, so `agent-sync`'s invalidation of a workflow on
 * every `workflow://*` event refreshes its graph too.
 */
export const workflowGraphKey = (workflowId: string) =>
  ['workflow', workflowId, 'graph'] as const

/** Node border per status (grey, cyan, green, red, amber) */
export const NODE_STATUS_CLASSES: Record<StepStatus, string> = {
  pending: 'border-border',
  running: 'border-info shadow-info/20',
  awaitingApproval: 'border-warning shadow-warning/20',
  done: 'border-success',
  failed: 'border-destructive',
  skipped: 'border-dashed border-border opacity-70',
}

/** Status pill styles */
export const STATUS_PILL_CLASSES: Record<StepStatus, string> = {
  pending: 'border-border bg-muted text-muted-foreground',
  running: 'border-info/30 bg-info/10 text-info',
  awaitingApproval: 'border-warning/30 bg-warning/10 text-warning',
  done: 'border-success/30 bg-success/10 text-success',
  failed: 'border-destructive/30 bg-destructive/10 text-destructive',
  skipped: 'border-border bg-muted text-muted-foreground',
}

type WorkflowEvent = Extract<
  NandiEvent,
  {
    event:
      | 'workflow://step_started'
      | 'workflow://step_awaiting_approval'
      | 'workflow://step_done'
      | 'workflow://done'
      | 'workflow://failed'
  }
>

export function isWorkflowRunEvent(event: NandiEvent): event is WorkflowEvent {
  return (
    event.event === 'workflow://step_started' ||
    event.event === 'workflow://step_awaiting_approval' ||
    event.event === 'workflow://step_done' ||
    event.event === 'workflow://done' ||
    event.event === 'workflow://failed'
  )
}

const resetNode = (node: GraphNode): GraphNode => ({
  ...node,
  status: 'pending',
  startedAt: null,
  endedAt: null,
  agentRunId: null,
  input: null,
  output: null,
  error: null,
})

/**
 * The graph after a `workflow://*` event, so the canvas updates before the
 * refetch lands. An event from a newer run resets every node first.
 * Events for other workflows leave the graph as it is.
 */
export function applyWorkflowEvent(
  graph: WorkflowGraph,
  event: NandiEvent,
  now: number = Date.now()
): WorkflowGraph {
  if (!isWorkflowRunEvent(event)) return graph
  const { payload } = event
  if (payload.workflowId !== graph.workflowId) return graph

  let nodes = graph.nodes
  if (payload.runId !== graph.runId) nodes = nodes.map(resetNode)
  const update = (stepId: string, patch: Partial<GraphNode>) => {
    nodes = nodes.map(node =>
      node.id === stepId ? { ...node, ...patch } : node
    )
  }
  const skipPending = () => {
    nodes = nodes.map(node =>
      node.status === 'pending' ? { ...node, status: 'skipped' } : node
    )
  }
  let runStatus = graph.runStatus

  switch (event.event) {
    case 'workflow://step_started':
      runStatus = 'running'
      update(event.payload.stepId, {
        status: 'running',
        agentRunId: event.payload.agentRunId,
        input: event.payload.input,
        startedAt: now,
        endedAt: null,
        output: null,
        error: null,
      })
      break
    case 'workflow://step_awaiting_approval':
      runStatus = 'awaitingApproval'
      update(event.payload.stepId, {
        status: 'awaitingApproval',
        input: event.payload.input,
      })
      break
    case 'workflow://step_done':
      update(event.payload.stepId, {
        status: 'done',
        output: event.payload.output,
        endedAt: now,
      })
      break
    case 'workflow://failed':
      runStatus = 'failed'
      if (event.payload.stepId) {
        update(event.payload.stepId, {
          status: 'failed',
          error: event.payload.error,
          endedAt: now,
        })
      }
      skipPending()
      break
    case 'workflow://done':
      runStatus = 'completed'
      skipPending()
      break
  }

  return { ...graph, nodes, runId: payload.runId, runStatus }
}

export const NODE_WIDTH = 220
export const NODE_HEIGHT = 76

export interface Positioned {
  id: string
  x: number
  y: number
}

/** Top-to-bottom positions (top-left corners) for every node */
export function layoutGraph(graph: WorkflowGraph): Positioned[] {
  const g = new dagre.graphlib.Graph()
  g.setGraph({ rankdir: 'TB', nodesep: 48, ranksep: 64, marginx: 16 })
  g.setDefaultEdgeLabel(() => ({}))
  for (const node of graph.nodes) {
    g.setNode(node.id, { width: NODE_WIDTH, height: NODE_HEIGHT })
  }
  for (const edge of graph.edges) g.setEdge(edge.from, edge.to)
  dagre.layout(g)
  return graph.nodes.map(node => {
    const { x, y } = g.node(node.id)
    return { id: node.id, x: x - NODE_WIDTH / 2, y: y - NODE_HEIGHT / 2 }
  })
}

/** `{{ step.output }}` references in a step's input */
const OUTPUT_REF = /\{\{\s*([A-Za-z0-9_-]+)\.output\s*\}\}/g

export function referencedSteps(template: string): string[] {
  return [...new Set([...template.matchAll(OUTPUT_REF)].map(m => m[1] ?? ''))]
}

export interface StepEdit {
  id: string
  agent: string
  input: string
  /** "Ask me first" */
  requiresApproval?: boolean
}

/**
 * The workflow's YAML with step `stepId` replaced by `edit`. A renamed step
 * keeps its place, and other steps' `{{old.output}}` follow the new ID.
 * Strings are written JSON-quoted (valid YAML); comments aren't kept.
 */
export function workflowYamlWithStep(
  workflow: Pick<Workflow, 'id' | 'name' | 'steps'> &
    Partial<Pick<Workflow, 'description'>>,
  stepId: string,
  edit: StepEdit
): string {
  const renamed = edit.id !== stepId
  const rename = (input: string) =>
    renamed
      ? input.replace(OUTPUT_REF, (match, ref: string) =>
          ref === stepId ? `{{${edit.id}.output}}` : match
        )
      : input
  const steps: WorkflowStep[] = workflow.steps.map(step =>
    step.id === stepId
      ? { ...step, ...edit }
      : {
          ...step,
          input: rename(step.input),
          instructions: step.instructions && rename(step.instructions),
          ask: step.ask && { ...step.ask, input: rename(step.ask.input) },
        }
  )
  return workflowToYaml({ ...workflow, steps })
}

/**
 * The workflow file for `workflow`: every field the domain reads, so
 * saving an edited step keeps the description and manual steps
 */
export function workflowToYaml(
  workflow: Pick<Workflow, 'id' | 'name' | 'steps'> &
    Partial<Pick<Workflow, 'description'>>
): string {
  const q = (value: string) => JSON.stringify(value)
  const lines = [`id: ${q(workflow.id)}`, `name: ${q(workflow.name)}`]
  if (workflow.description)
    lines.push(`description: ${q(workflow.description)}`)
  lines.push('steps:')
  for (const step of workflow.steps) {
    lines.push(`  - id: ${q(step.id)}`)
    if (step.manual) {
      lines.push('    step_type: manual')
      if (step.instructions)
        lines.push(`    instructions: ${q(step.instructions)}`)
      if (step.deadlineHours != null) {
        lines.push(`    deadline_hours: ${step.deadlineHours}`)
      }
      continue
    }
    if (step.ask) {
      // Asks a colleague (network v1): one person or capability
      lines.push('    ask:')
      for (const key of ['find', 'person', 'capability'] as const) {
        const value = step.ask[key]
        if (value) lines.push(`      ${key}: ${q(value)}`)
      }
      lines.push(`      input: ${q(step.ask.input)}`)
      if (step.ask.due) lines.push(`      due: ${q(step.ask.due)}`)
      if (step.requiresApproval) lines.push('    requires_approval: true')
      if (step.continueOnFailure) lines.push('    on_failure: continue')
      continue
    }
    lines.push(`    agent: ${q(step.agent)}`)
    lines.push(`    input: ${q(step.input)}`)
    if (step.requiresApproval) lines.push('    requires_approval: true')
    if (step.critical) lines.push('    critical: true')
    if (step.continueOnFailure) lines.push('    on_failure: continue')
  }
  return `${lines.join('\n')}\n`
}
