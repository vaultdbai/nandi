import { useTranslation } from 'react-i18next'
import { Container } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useMachineReport } from '@/hooks/useMachine'
import { openSettings } from '@/lib/settings-nav'

/**
 * Home's calm reminder until Docker answers: agents can talk, not act.
 * Shows nothing while checking, once Docker is ready, or if the check
 * itself failed.
 */
export function MachineHomeItem() {
  const { t } = useTranslation()
  const report = useMachineReport()
  if (!report.data || report.data.dockerReady) return null
  return (
    <p
      className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground"
      data-testid="home-machine"
    >
      <Container aria-hidden className="size-4" />
      {t('machine.home.text')}
      <Button
        size="sm"
        variant="link"
        className="h-auto p-0"
        onClick={() => openSettings('computer')}
      >
        {t('machine.prompt.setUp')}
      </Button>
    </p>
  )
}
