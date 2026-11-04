import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  useSetTrustPolicy,
  useSharedAgents,
  useTrustGraphHistory,
  useTrustGraphTeams,
  useTrustPolicies,
} from '@/hooks/useSharedAgents'
import type { TrustPolicy } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import {
  agentIdOf,
  buildTrustGraph,
  edgeWidth,
  layoutTrustGraph,
  neighborhood,
  type TrustEdge,
  type TrustGraphModel,
  type TrustNode,
  type TrustNodeKind,
} from './trust-graph-model'

const WIDTH = 800
const HEIGHT = 520
const NODE_RADIUS = 14

/** Theme colors per node kind (Tailwind palette variables) */
const NODE_FILL: Record<TrustNodeKind, string> = {
  mine: 'fill-success',
  callable: 'fill-info',
  caller: 'fill-warning',
  team: 'fill-muted-foreground',
}

const LEGEND_SWATCH: Record<TrustNodeKind, string> = {
  mine: 'bg-success',
  callable: 'bg-info',
  caller: 'bg-warning',
  team: 'bg-muted-foreground',
}

const POLICY_STROKE: Record<TrustPolicy, string> = {
  allow: 'stroke-success',
  deny: 'stroke-destructive',
  ask: 'stroke-muted-foreground',
}

type Selection = { kind: 'node'; id: string } | { kind: 'edge'; id: string }

/**
 * Force-directed graph of my agents, agents I call, agents that call me,
 * and teams; edges are weighted by calls in the last 30 days. Click a node
 * to highlight its connections, an inbound edge to set allow / deny / ask.
 */
export function TrustGraph() {
  const { t } = useTranslation()
  const { withMe, byMe } = useSharedAgents()
  const policies = useTrustPolicies()
  const history = useTrustGraphHistory()
  const teams = useTrustGraphTeams()
  const [selection, setSelection] = useState<Selection | null>(null)

  // Unavailable sources (not configured yet) just contribute nothing
  const model = buildTrustGraph({
    sharedByMe: byMe.data ?? [],
    sharedWithMe: withMe.data ?? [],
    history: history.data ?? [],
    teams: teams.data ?? [],
    policies: policies.data ?? [],
    // The window ends when the history was read
    now: history.dataUpdatedAt,
  })
  const positions = layoutTrustGraph(model, WIDTH, HEIGHT)
  const loading = byMe.isPending || withMe.isPending

  const highlighted =
    selection?.kind === 'node' ? neighborhood(model, selection.id) : null
  const selectedNode =
    selection?.kind === 'node'
      ? (model.nodes.find(node => node.id === selection.id) ?? null)
      : null
  const selectedEdge =
    selection?.kind === 'edge'
      ? (model.edges.find(edge => edge.id === selection.id) ?? null)
      : null

  if (!loading && model.nodes.length === 0) {
    return (
      <p className="p-6 text-sm text-muted-foreground">
        {t('shared.graph.empty')}
      </p>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-4 lg:flex-row">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
        <Legend />
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          className="w-full rounded-md border bg-card"
          role="group"
          aria-label={t('shared.graph.title')}
          onClick={() => setSelection(null)}
        >
          {model.edges.map(edge => {
            const from = positions.get(edge.source)
            const to = positions.get(edge.target)
            if (!from || !to) return null
            const dimmed =
              highlighted !== null &&
              !(highlighted.has(edge.source) && highlighted.has(edge.target))
            const selected = selectedEdge?.id === edge.id
            return (
              <g
                key={edge.id}
                role="button"
                tabIndex={0}
                aria-label={edgeLabel(edge, model, t)}
                data-testid={`trust-edge-${edge.id}`}
                className={cn(
                  'cursor-pointer outline-none',
                  dimmed && 'opacity-20'
                )}
                onClick={event => {
                  event.stopPropagation()
                  setSelection({ kind: 'edge', id: edge.id })
                }}
                onKeyDown={event => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault()
                    setSelection({ kind: 'edge', id: edge.id })
                  }
                }}
              >
                {/* Wide invisible hit area */}
                <line
                  x1={from.x}
                  y1={from.y}
                  x2={to.x}
                  y2={to.y}
                  className="stroke-transparent"
                  strokeWidth={12}
                />
                <line
                  x1={from.x}
                  y1={from.y}
                  x2={to.x}
                  y2={to.y}
                  className={cn(
                    edge.kind === 'team'
                      ? 'stroke-border'
                      : edge.policy
                        ? POLICY_STROKE[edge.policy]
                        : 'stroke-info/70',
                    selected && 'stroke-ring'
                  )}
                  strokeWidth={edgeWidth(edge.weight) + (selected ? 2 : 0)}
                  strokeDasharray={edge.kind === 'team' ? '4 4' : undefined}
                />
              </g>
            )
          })}
          {model.nodes.map(node => {
            const position = positions.get(node.id)
            if (!position) return null
            const dimmed = highlighted !== null && !highlighted.has(node.id)
            const selected = selectedNode?.id === node.id
            return (
              <g
                key={node.id}
                role="button"
                tabIndex={0}
                aria-label={`${node.label} (${t(`shared.graph.kind.${node.kind}`)})`}
                data-testid={`trust-node-${node.id}`}
                transform={`translate(${position.x} ${position.y})`}
                className={cn(
                  'cursor-pointer outline-none',
                  dimmed && 'opacity-30'
                )}
                onClick={event => {
                  event.stopPropagation()
                  setSelection({ kind: 'node', id: node.id })
                }}
                onKeyDown={event => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault()
                    setSelection({ kind: 'node', id: node.id })
                  }
                }}
              >
                {node.kind === 'team' ? (
                  <rect
                    x={-NODE_RADIUS}
                    y={-NODE_RADIUS}
                    width={NODE_RADIUS * 2}
                    height={NODE_RADIUS * 2}
                    rx={4}
                    className={cn(
                      NODE_FILL[node.kind],
                      selected ? 'stroke-ring' : 'stroke-background'
                    )}
                    strokeWidth={selected ? 3 : 2}
                  />
                ) : (
                  <circle
                    r={NODE_RADIUS}
                    className={cn(
                      NODE_FILL[node.kind],
                      selected ? 'stroke-ring' : 'stroke-background'
                    )}
                    strokeWidth={selected ? 3 : 2}
                  />
                )}
                <text
                  y={NODE_RADIUS + 14}
                  textAnchor="middle"
                  className="fill-foreground text-[11px]"
                >
                  {node.label}
                </text>
              </g>
            )
          })}
        </svg>
      </div>
      <aside className="w-full shrink-0 rounded-md border bg-card p-4 text-sm lg:w-72">
        {selectedNode && <NodeDetail node={selectedNode} model={model} />}
        {selectedEdge && <EdgeDetail edge={selectedEdge} model={model} />}
        {!selectedNode && !selectedEdge && (
          <p className="text-muted-foreground">{t('shared.graph.hint')}</p>
        )}
      </aside>
    </div>
  )
}

function Legend() {
  const { t } = useTranslation()
  const kinds: TrustNodeKind[] = ['mine', 'callable', 'caller', 'team']
  return (
    <ul className="flex flex-wrap gap-4 text-xs text-muted-foreground">
      {kinds.map(kind => (
        <li key={kind} className="flex items-center gap-1.5">
          <span className={cn('size-2.5 rounded-full', LEGEND_SWATCH[kind])} />
          {t(`shared.graph.kind.${kind}`)}
        </li>
      ))}
    </ul>
  )
}

const labelOf = (model: TrustGraphModel, id: string) =>
  model.nodes.find(node => node.id === id)?.label ?? agentIdOf(id)

function edgeLabel(
  edge: TrustEdge,
  model: TrustGraphModel,
  t: (key: string, options?: Record<string, unknown>) => string
) {
  return t(
    edge.kind === 'team' ? 'shared.graph.teamEdge' : 'shared.graph.callEdge',
    {
      from: labelOf(model, edge.source),
      to: labelOf(model, edge.target),
      n: edge.weight,
    }
  )
}

function NodeDetail({
  node,
  model,
}: {
  node: TrustNode
  model: TrustGraphModel
}) {
  const { t } = useTranslation()
  const connected = [...neighborhood(model, node.id)].filter(
    id => id !== node.id
  )
  return (
    <div className="flex flex-col gap-2" data-testid="trust-node-detail">
      <h3 className="font-semibold">{node.label}</h3>
      <div className="flex flex-wrap gap-1">
        <Badge variant="outline">{t(`shared.graph.kind.${node.kind}`)}</Badge>
        {node.detail && <Badge variant="secondary">{node.detail}</Badge>}
      </div>
      <p className="break-all text-xs text-muted-foreground">{node.id}</p>
      <p>{t('shared.graph.nodeCalls', { n: node.calls })}</p>
      <h4 className="text-xs font-medium text-muted-foreground">
        {t('shared.graph.connections', { n: connected.length })}
      </h4>
      <ul className="flex flex-col gap-1">
        {connected.map(id => (
          <li key={id} className="truncate">
            {labelOf(model, id)}
          </li>
        ))}
      </ul>
    </div>
  )
}

function EdgeDetail({
  edge,
  model,
}: {
  edge: TrustEdge
  model: TrustGraphModel
}) {
  const { t } = useTranslation()
  const setPolicy = useSetTrustPolicy()

  const change = (policy: TrustPolicy) => {
    setPolicy.mutate(
      {
        requester: edge.source,
        providerAgent: agentIdOf(edge.target),
        policy,
      },
      {
        onError: error =>
          toast.error(t('shared.graph.policyFailed', { error: error.message })),
      }
    )
  }

  return (
    <div className="flex flex-col gap-3" data-testid="trust-edge-detail">
      <h3 className="font-semibold">{edgeLabel(edge, model, t)}</h3>
      {edge.policy === null ? (
        <p className="text-muted-foreground">
          {edge.kind === 'team'
            ? t('shared.graph.teamEdgeHint')
            : t('shared.graph.outboundHint')}
        </p>
      ) : (
        <>
          <p className="text-muted-foreground">
            {t('shared.graph.policyHint')}
          </p>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            value={edge.policy}
            disabled={setPolicy.isPending}
            onValueChange={value => {
              if (value && value !== edge.policy) change(value as TrustPolicy)
            }}
            aria-label={t('shared.graph.policy')}
          >
            {(['allow', 'ask', 'deny'] as const).map(policy => (
              <ToggleGroupItem key={policy} value={policy}>
                {t(`shared.graph.policies.${policy}`)}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </>
      )}
    </div>
  )
}
