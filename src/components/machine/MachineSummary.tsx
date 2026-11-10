import { useTranslation } from 'react-i18next'
import type { MachineReport } from '@/lib/bindings'
import { plainError } from '@/lib/toast-error'

/** One line over the checklist: ready, ready to act, or talk-only */
export function MachineSummary({
  report,
  error,
}: {
  report: MachineReport | undefined
  error?: unknown
}) {
  const { t } = useTranslation()
  if (error)
    return (
      <p className="text-sm text-muted-foreground" role="status">
        {t('machine.checkFailed', { error: plainError(error) })}
      </p>
    )
  if (!report) return null
  const key = report.ready
    ? 'machine.summary.ready'
    : report.dockerReady
      ? 'machine.summary.canAct'
      : 'machine.summary.talkOnly'
  return (
    <p className="text-sm" role="status" data-testid="machine-summary">
      {t(key)}
    </p>
  )
}
