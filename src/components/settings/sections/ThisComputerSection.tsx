import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { MachineChecklist } from '@/components/machine/MachineChecklist'
import { MachineSummary } from '@/components/machine/MachineSummary'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import {
  useCheckMachineAgain,
  useMachineReport,
  useMachineSettings,
  useSetStartDockerOnOpen,
} from '@/hooks/useMachine'
import { SettingsSectionFrame } from '../SettingsFrame'

/**
 * Settings › This computer: is it ready for agents to act? The checklist
 * re-checks every 10 s while open; Check again re-runs everything now.
 * Below it, whether Docker starts when Nandi opens (Windows and macOS).
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
      {(report.data?.os === 'windows' || report.data?.os === 'macos') && (
        <StartOnOpenSwitch />
      )}
    </SettingsSectionFrame>
  )
}

function StartOnOpenSwitch() {
  const { t } = useTranslation()
  const settings = useMachineSettings()
  const set = useSetStartDockerOnOpen()
  return (
    <Field orientation="horizontal">
      <Switch
        id="start-docker-on-open"
        checked={settings.data?.startDockerOnOpen === true}
        disabled={!settings.data || set.isPending}
        onCheckedChange={start => set.mutate(start)}
      />
      <div className="space-y-0.5">
        <FieldLabel htmlFor="start-docker-on-open">
          {t('machine.startOnOpen.label')}
        </FieldLabel>
        <FieldDescription>{t('machine.startOnOpen.hint')}</FieldDescription>
      </div>
    </Field>
  )
}
