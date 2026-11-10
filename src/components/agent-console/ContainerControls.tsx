import { useTranslation } from 'react-i18next'
import { TriangleAlert, X } from 'lucide-react'
import { DockerSetupPrompt } from '@/components/machine/DockerSetupPrompt'
import { isToolsUnavailable } from '@/components/machine/machine-model'
import { Button } from '@/components/ui/button'
import { useAgentStore } from '@/store/agent-store'

/**
 * Shown after `agent://warning`. "Tool execution unavailable" (Docker
 * isn't ready, so the agent answers without tools) becomes the calm
 * "Agents need Docker to act. Set up"; other warnings show as they are.
 */
export function AgentWarningBanner({
  agentId,
  warning,
}: {
  agentId: string
  warning: string
}) {
  const { t } = useTranslation()
  const dismiss = () => useAgentStore.getState().setAgentWarning(agentId, null)

  if (isToolsUnavailable(warning))
    return <DockerSetupPrompt onDismiss={dismiss} />

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
        onClick={dismiss}
      >
        <X />
      </Button>
    </div>
  )
}
