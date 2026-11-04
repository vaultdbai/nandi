import type { RunSummary } from '@/lib/tauri-bindings'
import type { AgentInfo, AgentStatus } from '@/types/agent'

interface AgentStatusStyle {
  /** Solid indicator dot */
  dotClassName: string
  /** Tinted pill used in headers */
  pillClassName: string
  labelKey: `agentConsole.status.${AgentStatus}`
}

/** Single source of truth for how each agent status is presented */
export const AGENT_STATUS_STYLES: Record<AgentStatus, AgentStatusStyle> = {
  running: {
    dotClassName: 'bg-success',
    pillClassName: 'border-success/30 bg-success/10 text-success',
    labelKey: 'agentConsole.status.running',
  },
  waitingApproval: {
    dotClassName: 'bg-warning',
    pillClassName: 'border-warning/30 bg-warning/10 text-warning',
    labelKey: 'agentConsole.status.waitingApproval',
  },
  idle: {
    dotClassName: 'bg-muted-foreground/50',
    pillClassName: 'border-border bg-muted text-muted-foreground',
    labelKey: 'agentConsole.status.idle',
  },
  error: {
    dotClassName: 'bg-destructive',
    pillClassName: 'border-destructive/30 bg-destructive/10 text-destructive',
    labelKey: 'agentConsole.status.error',
  },
}

/**
 * An agent's status with its live run applied: the store's current run is
 * newer than the status `list_agents` reported.
 */
export function effectiveStatus(
  agent: AgentInfo,
  run: RunSummary | null
): AgentStatus {
  if (agent.status === 'error') return 'error'
  if (run?.status === 'awaitingApproval') return 'waitingApproval'
  if (run) return 'running'
  return agent.status === 'running' ? 'running' : 'idle'
}
