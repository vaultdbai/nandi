/**
 * Test helpers for the workflow builder: jsdom stand-ins React Flow needs to
 * measure nodes, a sample graph, and mocks for the builder's commands (the
 * shared test setup doesn't stub them).
 */

import { vi } from 'vitest'
import type { GraphNode, Workflow, WorkflowGraph } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'

/** React Flow measures nodes with ResizeObserver, DOMMatrix, and offsets */
export function mockReactFlowDom() {
  globalThis.ResizeObserver = class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) {
      const contentRect = { width: 1, height: 1 } as DOMRectReadOnly
      this.callback([{ target, contentRect } as ResizeObserverEntry], this)
    }
    unobserve() {
      // Nothing to release
    }
    disconnect() {
      // Nothing to release
    }
  }
  globalThis.DOMMatrixReadOnly ??= class {
    m22: number
    constructor(transform?: string) {
      const scale = /scale\(([0-9.]+)\)/.exec(transform ?? '')?.[1]
      this.m22 = scale !== undefined ? Number(scale) : 1
    }
  } as unknown as typeof DOMMatrixReadOnly
  Object.defineProperties(HTMLElement.prototype, {
    offsetHeight: {
      configurable: true,
      get(this: HTMLElement) {
        return parseFloat(this.style.height) || 1
      },
    },
    offsetWidth: {
      configurable: true,
      get(this: HTMLElement) {
        return parseFloat(this.style.width) || 1
      },
    },
  })
  Object.assign(SVGElement.prototype, {
    getBBox: () => ({ x: 0, y: 0, width: 0, height: 0 }),
  })
}

export function graphNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    id: 'research',
    name: 'research',
    agentId: 'research-agent',
    agentName: 'Research Agent',
    status: 'pending',
    requiresApproval: false,
    startedAt: null,
    endedAt: null,
    agentRunId: null,
    inputTemplate: "Find today's AI news",
    input: null,
    output: null,
    error: null,
    critical: false,
    continueOnFailure: false,
    ...overrides,
  }
}

const T0 = Date.UTC(2026, 9, 1, 12)

/** `daily-report`: research (done) → publish (needs approval) */
export function sampleGraph(
  overrides: Partial<WorkflowGraph> = {}
): WorkflowGraph {
  return {
    workflowId: 'daily-report',
    runId: 'wr-1',
    runStatus: 'completed',
    nodes: [
      graphNode({
        status: 'done',
        startedAt: T0,
        endedAt: T0 + 2_000,
        agentRunId: 'run-a',
        input: "Find today's AI news",
        output: '**Three** stories',
      }),
      graphNode({
        id: 'publish',
        name: 'publish',
        agentId: 'publisher-agent',
        agentName: 'Publisher',
        requiresApproval: true,
        inputTemplate: 'Send: {{research.output}}',
      }),
    ],
    edges: [{ from: 'research', to: 'publish', kind: 'sequence' }],
    ...overrides,
  }
}

/** Stubs `getWorkflowGraph` / `saveWorkflowYaml`; returns the mocks */
export function mockBuilderCommands(graph: WorkflowGraph = sampleGraph()) {
  const getWorkflowGraph = vi
    .fn()
    .mockResolvedValue({ status: 'ok', data: graph })
  const saveWorkflowYaml = vi.fn()
  Object.assign(commands, { getWorkflowGraph, saveWorkflowYaml })
  return { getWorkflowGraph, saveWorkflowYaml }
}

/** The `daily-report` definition matching `sampleGraph` (no runs) */
export function sampleWorkflow(): Workflow {
  return {
    id: 'daily-report',
    name: 'Daily Report',
    path: '/repo/workflows/daily-report.yaml',
    description: null,
    source: 'id: daily-report\nname: Daily Report\n',
    steps: [
      {
        id: 'research',
        agent: 'research-agent',
        input: "Find today's AI news",
        requiresApproval: false,
      },
      {
        id: 'publish',
        agent: 'publisher-agent',
        input: 'Send: {{research.output}}',
        requiresApproval: true,
      },
    ],
    runs: [],
  }
}
