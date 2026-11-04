import { useTranslation } from 'react-i18next'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import {
  CheckCircle2,
  Circle,
  Loader2,
  MinusCircle,
  ShieldAlert,
  XCircle,
} from 'lucide-react'
import type { GraphNode } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import {
  NODE_HEIGHT,
  NODE_STATUS_CLASSES,
  NODE_WIDTH,
  stepStatus,
  type StepStatus,
} from './graph-model'

export type StepNodeType = Node<{ step: GraphNode }, 'step'>

const ICONS: Record<StepStatus, typeof Circle> = {
  pending: Circle,
  running: Loader2,
  awaitingApproval: ShieldAlert,
  done: CheckCircle2,
  failed: XCircle,
  skipped: MinusCircle,
}

const ICON_CLASSES: Record<StepStatus, string> = {
  pending: 'text-muted-foreground',
  running: 'animate-spin text-info',
  awaitingApproval: 'text-warning',
  done: 'text-success',
  failed: 'text-destructive',
  skipped: 'text-muted-foreground',
}

/** Tinted square behind the status icon */
const TILE_CLASSES: Record<StepStatus, string> = {
  pending: 'bg-muted',
  running: 'bg-info/15',
  awaitingApproval: 'bg-warning/15',
  done: 'bg-success/15',
  failed: 'bg-destructive/15',
  skipped: 'bg-muted',
}

/** A step on the canvas: a card with a status-coloured border and icon */
export function StepNode({ data, selected }: NodeProps<StepNodeType>) {
  const { t } = useTranslation()
  const { step } = data
  const status = stepStatus(step)
  const Icon = ICONS[status]
  const label = t(`workflowBuilder.status.${status}`)

  return (
    <div
      data-status={status}
      style={{ width: NODE_WIDTH, height: NODE_HEIGHT }}
      className={cn(
        'flex items-center gap-3 rounded-xl border-2 bg-card px-3 text-card-foreground shadow-md shadow-black/5 transition-all',
        NODE_STATUS_CLASSES[status],
        status === 'running' && 'shadow-lg',
        selected && 'ring-2 ring-ring ring-offset-2 ring-offset-background'
      )}
    >
      <Handle
        type="target"
        position={Position.Top}
        isConnectable={false}
        className="border-border! bg-muted-foreground!"
      />
      <span
        className={cn(
          'flex size-10 shrink-0 items-center justify-center rounded-lg',
          TILE_CLASSES[status]
        )}
      >
        <Icon
          aria-label={label}
          role="img"
          className={cn('size-5', ICON_CLASSES[status])}
        />
      </span>
      <div className="min-w-0 flex-1 text-start">
        <p className="truncate text-sm font-semibold">{step.name}</p>
        <p className="truncate text-xs text-muted-foreground">
          {step.agentName ?? step.agentId}
        </p>
        {(step.requiresApproval || status !== 'pending') && (
          // Status in words, not only colour; the approval rule too
          <p
            className={cn(
              'truncate text-[11px] font-medium',
              status === 'pending'
                ? 'text-warning'
                : ICON_CLASSES[status].replace('animate-spin', '')
            )}
          >
            {[
              step.requiresApproval ? t('workflows.requiresApproval') : null,
              status !== 'pending' ? label : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          </p>
        )}
      </div>
      <Handle
        type="source"
        position={Position.Bottom}
        isConnectable={false}
        className="border-border! bg-muted-foreground!"
      />
    </div>
  )
}
