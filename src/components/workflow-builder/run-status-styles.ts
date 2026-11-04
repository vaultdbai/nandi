/**
 * Workflow run status presentation: per-status icons and colours, shared
 * by the run summary, the transcript, and the run history.
 */
import {
  Ban,
  CheckCircle2,
  Circle,
  Loader2,
  ShieldAlert,
  UserRoundCheck,
  XCircle,
} from 'lucide-react'
import type {
  WorkflowRun,
  WorkflowStepRun,
  WorkflowStepStatus,
} from '@/lib/tauri-bindings'

type WorkflowRunStatus = WorkflowRun['status']

export interface StatusStyle {
  icon: typeof Circle
  /** Icon colour (and spin for running) */
  iconClassName: string
  /** Tinted square behind the icon */
  tileClassName: string
  /** Solid fill: progress segments, rail dots */
  fillClassName: string
  /** Tinted pill */
  pillClassName: string
}

export const STEP_STATUS_STYLES: Record<WorkflowStepStatus, StatusStyle> = {
  pending: {
    icon: Circle,
    iconClassName: 'text-muted-foreground',
    tileClassName: 'bg-muted text-muted-foreground',
    fillClassName: 'bg-muted-foreground/25',
    pillClassName: 'border-border bg-muted text-muted-foreground',
  },
  running: {
    icon: Loader2,
    iconClassName: 'animate-spin text-info',
    tileClassName: 'bg-info/15 text-info',
    fillClassName: 'bg-info animate-pulse',
    pillClassName: 'border-info/30 bg-info/10 text-info',
  },
  awaitingApproval: {
    icon: ShieldAlert,
    iconClassName: 'text-warning',
    tileClassName: 'bg-warning/15 text-warning',
    fillClassName: 'bg-warning',
    pillClassName: 'border-warning/30 bg-warning/10 text-warning',
  },
  awaitingInput: {
    icon: UserRoundCheck,
    iconClassName: 'text-warning',
    tileClassName: 'bg-warning/15 text-warning',
    fillClassName: 'bg-warning',
    pillClassName: 'border-warning/30 bg-warning/10 text-warning',
  },
  done: {
    icon: CheckCircle2,
    iconClassName: 'text-success',
    tileClassName: 'bg-success/15 text-success',
    fillClassName: 'bg-success',
    pillClassName: 'border-success/30 bg-success/10 text-success',
  },
  failed: {
    icon: XCircle,
    iconClassName: 'text-destructive',
    tileClassName: 'bg-destructive/15 text-destructive',
    fillClassName: 'bg-destructive',
    pillClassName: 'border-destructive/30 bg-destructive/10 text-destructive',
  },
  rejected: {
    icon: Ban,
    iconClassName: 'text-destructive',
    tileClassName: 'bg-destructive/15 text-destructive',
    fillClassName: 'bg-destructive/70',
    pillClassName: 'border-destructive/30 bg-destructive/10 text-destructive',
  },
}

/** A run's overall status, in the same colours as its steps */
const RUN_STATUS_STEP: Record<WorkflowRunStatus, WorkflowStepStatus> = {
  running: 'running',
  awaitingApproval: 'awaitingApproval',
  completed: 'done',
  failed: 'failed',
  paused: 'awaitingApproval',
  waiting: 'awaitingInput',
  cancelled: 'rejected',
}

export function runStatusStyle(status: WorkflowRunStatus): StatusStyle {
  return STEP_STATUS_STYLES[RUN_STATUS_STEP[status]]
}

/** Card wash per run status: a soft gradient from the status colour */
export const RUN_WASH: Record<WorkflowRunStatus, string> = {
  running: 'from-info/12 border-info/30',
  awaitingApproval: 'from-warning/12 border-warning/30',
  completed: 'from-success/12 border-success/30',
  failed: 'from-destructive/12 border-destructive/30',
  paused: 'from-warning/12 border-warning/30',
  waiting: 'from-warning/12 border-warning/30',
  cancelled: 'from-muted border-border',
}

/** How long the step ran, or has been running at `now` */
export function stepDuration(
  step: WorkflowStepRun,
  now: number
): number | null {
  if (step.startedAt === null) return null
  return (step.endedAt ?? now) - step.startedAt
}
