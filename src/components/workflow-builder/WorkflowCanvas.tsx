import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Background,
  Controls,
  MarkerType,
  Position,
  ReactFlow,
  type Edge,
  type NodeTypes,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { Skeleton } from '@/components/ui/skeleton'
import { useTheme } from '@/hooks/use-theme'
import type { WorkflowGraph } from '@/lib/bindings'
import { layoutGraph, NODE_HEIGHT, NODE_WIDTH } from './graph-model'
import { StepDetailsPanel } from './StepDetailsPanel'
import { StepNode, type StepNodeType } from './StepNode'
import { useWorkflowGraph } from './useWorkflowGraph'

const NODE_TYPES: NodeTypes = { step: StepNode }

// Every node has the same size and handles, so edges draw without waiting
// for the DOM measurement
const HANDLES = [
  { type: 'target' as const, position: Position.Top, x: NODE_WIDTH / 2, y: 0 },
  {
    type: 'source' as const,
    position: Position.Bottom,
    x: NODE_WIDTH / 2,
    y: NODE_HEIGHT,
  },
]

interface WorkflowCanvasProps {
  workflowId: string
  /**
   * Controlled selection. Without `onSelectStep` the canvas is standalone:
   * it keeps its own selection and shows the step details beside itself.
   */
  selectedStepId?: string | null
  onSelectStep?: (stepId: string | null) => void
}

/** Node graph of a workflow's steps with live status. */
export function WorkflowCanvas({
  workflowId,
  selectedStepId,
  onSelectStep,
}: WorkflowCanvasProps) {
  const [ownSelection, setOwnSelection] = useState<string | null>(null)
  const standalone = onSelectStep === undefined
  const selected = standalone ? ownSelection : (selectedStepId ?? null)
  const select = onSelectStep ?? setOwnSelection

  const canvas = (
    <GraphPane
      workflowId={workflowId}
      selectedStepId={selected}
      onSelectStep={select}
    />
  )
  if (!standalone) return canvas
  return (
    <div className="flex h-full min-h-0">
      <div className="min-w-0 flex-[7]">{canvas}</div>
      <aside className="min-w-0 flex-[3] border-s">
        <StepDetailsPanel
          workflowId={workflowId}
          stepId={selected}
          onClose={() => setOwnSelection(null)}
        />
      </aside>
    </div>
  )
}

function GraphPane({
  workflowId,
  selectedStepId,
  onSelectStep,
}: {
  workflowId: string
  selectedStepId: string | null
  onSelectStep: (stepId: string | null) => void
}) {
  const { t } = useTranslation()
  const { data: graph, isLoading, error } = useWorkflowGraph(workflowId)

  if (isLoading) {
    return (
      <div className="p-6">
        <Skeleton className="h-64" />
      </div>
    )
  }
  if (error || !graph) {
    return (
      <p role="alert" className="p-6 text-sm text-destructive">
        {typeof error === 'string' ? error : t('workflowBuilder.loadError')}
      </p>
    )
  }
  return (
    <GraphFlow
      graph={graph}
      selectedStepId={selectedStepId}
      onSelectStep={onSelectStep}
    />
  )
}

/**
 * Edges light up as the run moves: green between finished steps, blue
 * into the running one, amber into one waiting for approval.
 */
function edgeColor(from: string | undefined, to: string | undefined): string {
  if (to === 'running') return 'var(--info)'
  if (to === 'awaitingApproval') return 'var(--warning)'
  if (to === 'failed') return 'var(--destructive)'
  if (from === 'done' && to === 'done') return 'var(--success)'
  return 'var(--muted-foreground)'
}

function GraphFlow({
  graph,
  selectedStepId,
  onSelectStep,
}: {
  graph: WorkflowGraph
  selectedStepId: string | null
  onSelectStep: (stepId: string | null) => void
}) {
  const { t } = useTranslation()
  const { theme } = useTheme()
  const positions = new Map(layoutGraph(graph).map(p => [p.id, p]))
  const statusOf = new Map(graph.nodes.map(n => [n.id, n.status]))

  const nodes: StepNodeType[] = graph.nodes.map(step => ({
    id: step.id,
    type: 'step',
    position: {
      x: positions.get(step.id)?.x ?? 0,
      y: positions.get(step.id)?.y ?? 0,
    },
    data: { step },
    width: NODE_WIDTH,
    height: NODE_HEIGHT,
    handles: HANDLES,
    selected: step.id === selectedStepId,
    draggable: false,
    connectable: false,
    ariaLabel: step.name,
  }))
  const nameOf = new Map(graph.nodes.map(node => [node.id, node.name]))
  const edges: Edge[] = graph.edges.map(edge => {
    const color = edgeColor(statusOf.get(edge.from), statusOf.get(edge.to))
    return {
      id: `${edge.from}->${edge.to}`,
      source: edge.from,
      target: edge.to,
      type: 'smoothstep',
      // Flowing towards the step that's running now
      animated: statusOf.get(edge.to) === 'running',
      markerEnd: { type: MarkerType.ArrowClosed, color },
      className: edge.kind === 'data' ? 'opacity-60' : undefined,
      style: {
        stroke: color,
        strokeWidth: 2,
        ...(edge.kind === 'data' ? { strokeDasharray: '4 4' } : {}),
      },
      ariaLabel: t('workflowBuilder.edgeLabel', {
        from: nameOf.get(edge.from) ?? edge.from,
        to: nameOf.get(edge.to) ?? edge.to,
      }),
    }
  })

  return (
    <div
      className="h-full min-h-80 w-full"
      aria-label={t('workflowBuilder.canvasLabel')}
      role="region"
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        colorMode={theme}
        fitView
        fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
        nodesDraggable={false}
        nodesConnectable={false}
        edgesFocusable={false}
        proOptions={{ hideAttribution: true }}
        onNodeClick={(_, node) => onSelectStep(node.id)}
        onPaneClick={() => onSelectStep(null)}
      >
        <Background gap={20} size={1.5} />
        <Controls showInteractive={false} />
      </ReactFlow>
      {/* The arrows as text, for screen readers (edges aren't focusable) */}
      <ul className="sr-only" aria-label={t('workflowBuilder.dependencies')}>
        {edges.map(edge => (
          <li key={edge.id}>{edge.ariaLabel}</li>
        ))}
      </ul>
    </div>
  )
}
