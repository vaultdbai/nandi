import { useTranslation } from 'react-i18next'
import {
  BACKFILL_RANGES,
  estimateTotals,
} from '@/components/settings/email/email-model'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { useBackfillEstimate } from '@/hooks/useEmail'
import type { BackfillRange } from '@/lib/tauri-bindings'

/**
 * "Also read older mail in the background": how far back, with an
 * estimate of the emails and Chief of Staff runs once a mailbox is
 * connected
 */
export function BackfillChoice({
  value,
  onChange,
  connected,
  idPrefix,
}: {
  value: BackfillRange
  onChange: (range: BackfillRange) => void
  connected: boolean
  idPrefix: string
}) {
  const { t } = useTranslation()
  const estimate = useBackfillEstimate(value, connected)
  const totals = estimate.data ? estimateTotals(estimate.data) : null
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-2 text-sm font-medium">
        {t('email.backfill.title')}
      </legend>
      <RadioGroup
        value={value}
        onValueChange={range => onChange(range as BackfillRange)}
        className="flex flex-wrap gap-2"
      >
        {BACKFILL_RANGES.map(range => {
          const id = `${idPrefix}-backfill-${range}`
          return (
            <div
              key={range}
              className="flex items-center gap-2 rounded-md border px-3 py-2"
            >
              <RadioGroupItem id={id} value={range} />
              <Label htmlFor={id}>{t(`email.backfill.range.${range}`)}</Label>
            </div>
          )
        })}
      </RadioGroup>
      <p className="text-xs text-muted-foreground">
        {value === 'off'
          ? t('email.backfill.offHelp')
          : totals && totals.emails > 0
            ? t('email.backfill.estimate', totals)
            : t('email.backfill.help')}
      </p>
    </fieldset>
  )
}
