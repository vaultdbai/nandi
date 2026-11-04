import { useTranslation } from 'react-i18next'
import { TriangleAlert, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useAgentStore } from '@/store/agent-store'

/** Shown after `agent://warning`, e.g. the agent is answering without tools. */
export function AgentWarningBanner({
  agentId,
  warning,
}: {
  agentId: string
  warning: string
}) {
  const { t } = useTranslation()

  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b border-warning/30 bg-warning/10 px-4 py-1.5 text-xs text-warning"
    >
      <TriangleAlert className="size-3.5 shrink-0" />
      <span className="min-w-0 flex-1 truncate" title={warning}>
        {warning}
      </span>
      <Button
        size="icon-sm"
        variant="ghost"
        className="size-6"
        aria-label={t('agentConsole.container.dismissWarning')}
        title={t('agentConsole.container.dismissWarning')}
        onClick={() => useAgentStore.getState().setAgentWarning(agentId, null)}
      >
        <X />
      </Button>
    </div>
  )
}
