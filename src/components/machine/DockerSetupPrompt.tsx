import { useTranslation } from 'react-i18next'
import { Container, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { openSettings } from '@/lib/settings-nav'
import { cn } from '@/lib/utils'

/**
 * "Agents need Docker to act. Set up": a calm line wherever a run would
 * need tools and Docker isn't ready (the agent page replaces the core's
 * "Tool execution unavailable" warning with it). Set up opens Settings ›
 * This computer.
 */
export function DockerSetupPrompt({
  onDismiss,
  className,
}: {
  onDismiss?: () => void
  className?: string
}) {
  const { t } = useTranslation()
  return (
    <div
      role="status"
      data-testid="docker-setup-prompt"
      className={cn(
        'flex shrink-0 items-center gap-2 border-b bg-muted/40 px-4 py-1.5 text-xs text-muted-foreground',
        className
      )}
    >
      <Container aria-hidden className="size-3.5 shrink-0" />
      <span className="min-w-0 flex-1">{t('machine.prompt.text')}</span>
      <Button
        size="sm"
        variant="link"
        className="h-auto p-0 text-xs"
        onClick={() => openSettings('computer')}
      >
        {t('machine.prompt.setUp')}
      </Button>
      {onDismiss && (
        <Button
          size="icon-sm"
          variant="ghost"
          className="size-6"
          aria-label={t('agentConsole.container.dismissWarning')}
          title={t('agentConsole.container.dismissWarning')}
          onClick={onDismiss}
        >
          <X />
        </Button>
      )}
    </div>
  )
}
