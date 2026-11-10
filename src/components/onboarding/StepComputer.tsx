import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { MachineChecklist } from '@/components/machine/MachineChecklist'
import { MachineSummary } from '@/components/machine/MachineSummary'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useCheckMachineAgain, useMachineReport } from '@/hooks/useMachine'

export interface StepComputerProps {
  onNext: () => void
  onBack: () => void
}

/**
 * Onboarding: get your computer ready. Strongly recommended, never
 * required: without Docker, agents can only talk, not act. Skipping leaves
 * the reminder on Home until Docker answers.
 */
export function StepComputer({ onNext, onBack }: StepComputerProps) {
  const { t } = useTranslation()
  const report = useMachineReport({ poll: true })
  const again = useCheckMachineAgain()
  const dockerReady = report.data?.dockerReady === true
  return (
    <div className="flex flex-col gap-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">
          {t('onboarding.computer.title')}
        </h2>
        <p className="text-sm text-muted-foreground">
          {t('onboarding.computer.description')}
        </p>
        <p className="text-sm font-medium">
          {t('onboarding.computer.withoutDocker')}
        </p>
      </div>
      <MachineSummary report={report.data} error={report.error} />
      <MachineChecklist report={report.data} />
      <div className="flex items-center justify-between gap-2">
        <Button variant="ghost" onClick={onBack}>
          {t('onboarding.back')}
        </Button>
        <div className="flex gap-2">
          <Button
            variant="outline"
            disabled={again.isPending}
            onClick={() => again.mutate()}
          >
            {again.isPending ? <Spinner /> : <RefreshCw />}
            {t('machine.action.checkAgain')}
          </Button>
          {dockerReady ? (
            <Button onClick={onNext}>{t('onboarding.continue')}</Button>
          ) : (
            <Button variant="secondary" onClick={onNext}>
              {t('onboarding.computer.skip')}
            </Button>
          )}
        </div>
      </div>
    </div>
  )
}
