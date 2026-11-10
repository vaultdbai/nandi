import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { MachineChecklist } from '@/components/machine/MachineChecklist'
import { MachineSummary } from '@/components/machine/MachineSummary'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useCheckMachineAgain, useMachineReport } from '@/hooks/useMachine'
import { SettingsSectionFrame } from '../SettingsFrame'

/**
 * Settings › This computer: is it ready for agents to act? The checklist
 * re-checks every 10 s while open; Check again re-runs everything now.
 */
export function ThisComputerSection() {
  const { t } = useTranslation()
  const report = useMachineReport({ poll: true })
  const again = useCheckMachineAgain()
  return (
    <SettingsSectionFrame
      title={t('settings.section.computer')}
      description={t('machine.description')}
      actions={
        <Button
          size="sm"
          variant="outline"
          disabled={again.isPending}
          onClick={() => again.mutate()}
        >
          {again.isPending ? <Spinner /> : <RefreshCw />}
          {t('machine.action.checkAgain')}
        </Button>
      }
    >
      <MachineSummary report={report.data} error={report.error} />
      <MachineChecklist report={report.data} />
    </SettingsSectionFrame>
  )
}
