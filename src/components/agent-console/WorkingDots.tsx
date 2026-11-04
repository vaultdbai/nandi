import { useTranslation } from 'react-i18next'
import { useNow } from '@/components/agents/task-utils'
import { formatDuration } from '@/lib/format-time'
import type { AgentInfo } from '@/types/agent'
import { AgentAvatar } from './AgentAvatar'

/**
 * Animated "." → ".." → "..." after a label, so a wait never looks frozen.
 * Shows a still "..." when the user prefers reduced motion (App.css).
 */
export function WorkingDots({ label }: { label?: string }) {
  return (
    <span className="inline-flex items-baseline">
      {label && <span>{label}</span>}
      <span aria-hidden className="working-dots">
        ...
      </span>
    </span>
  )
}

/** A short wait needs no clock; past this, the row says how long */
const SHOW_ELAPSED_MS = 5000

/**
 * The agent's row at the end of a conversation while it works on a turn
 * and nothing has streamed yet: its avatar, its name, the dots and, after
 * a few seconds, how long the turn has taken since `since` (epoch ms).
 */
export function WorkingRow({
  agent,
  since,
}: {
  agent: AgentInfo
  since?: number
}) {
  const { t } = useTranslation()
  const now = useNow(since !== undefined)
  const elapsed = since === undefined ? 0 : now - since
  return (
    <div
      role="status"
      aria-label={t('agentConsole.chat.working', { name: agent.name })}
      className="flex gap-3"
    >
      <AgentAvatar agent={agent} />
      <div className="min-w-0 flex-1 space-y-1.5">
        <span className="text-sm font-medium text-foreground">
          {agent.name}
        </span>
        <p className="text-sm text-muted-foreground">
          <WorkingDots label={t('agentConsole.chat.workingLabel')} />
          {elapsed >= SHOW_ELAPSED_MS && (
            <span className="ms-2 text-xs tabular-nums">
              {formatDuration(elapsed)}
            </span>
          )}
        </p>
      </div>
    </div>
  )
}
