import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight, Plus, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useCalendarPreview } from '@/hooks/useTriggers'
import { cn } from '@/lib/utils'
import {
  calendarConfig,
  WEEKDAYS,
  type CoverRow,
  type TriggerFormState,
} from '@/lib/trigger-form'

interface CalendarFieldsProps {
  form: TriggerFormState
  onChange: (patch: Partial<TriggerFormState>) => void
}

const EMPTY_COVER: CoverRow = { person: '', from: '', to: '', backup: '' }

/**
 * A calendar: times and days, who is responsible (taking turns, fixed
 * weekdays, stand-ins), their backup, and the next runs with who is on duty
 */
export function CalendarFields({ form, onChange }: CalendarFieldsProps) {
  const { t } = useTranslation()
  const id = useId()

  const setCover = (index: number, patch: Partial<CoverRow>) =>
    onChange({
      cover: form.cover.map((row, i) =>
        i === index ? { ...row, ...patch } : row
      ),
    })

  return (
    <div className="flex flex-col gap-5">
      <div className="grid gap-4 sm:grid-cols-2">
        <Field
          label={t('triggers.calendar.times')}
          htmlFor={`${id}-times`}
          hint={t('triggers.calendar.timesHint')}
        >
          <Input
            id={`${id}-times`}
            dir="ltr"
            className="font-mono"
            value={form.times}
            placeholder="09:00, 17:00"
            onChange={event => onChange({ times: event.target.value })}
          />
        </Field>
        <Field
          label={t('triggers.calendar.skipDates')}
          htmlFor={`${id}-skip`}
          hint={t('triggers.calendar.skipDatesHint')}
        >
          <Input
            id={`${id}-skip`}
            dir="ltr"
            className="font-mono"
            value={form.skipDates}
            placeholder="2026-12-25, 2027-01-01"
            onChange={event => onChange({ skipDates: event.target.value })}
          />
        </Field>
      </div>

      <fieldset className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <legend className="mb-1.5 text-sm font-medium">
          {t('triggers.calendar.days')}
        </legend>
        {WEEKDAYS.map(day => (
          <label key={day} className="flex items-center gap-2 text-sm">
            <Checkbox
              checked={form.days.includes(day)}
              onCheckedChange={checked =>
                onChange({
                  days:
                    checked === true
                      ? WEEKDAYS.filter(d => d === day || form.days.includes(d))
                      : form.days.filter(d => d !== day),
                })
              }
            />
            {t(`triggers.calendar.weekday.${day}`)}
          </label>
        ))}
      </fieldset>

      <section className="flex flex-col gap-4 rounded-md border p-3">
        <div>
          <h3 className="text-sm font-medium">
            {t('triggers.calendar.whoTitle')}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t('triggers.calendar.whoHint')}
          </p>
        </div>
        <div className="grid gap-4 sm:grid-cols-3">
          <Field
            label={t('triggers.calendar.people')}
            htmlFor={`${id}-people`}
            className="sm:col-span-3"
          >
            <Input
              id={`${id}-people`}
              value={form.people}
              placeholder="alice, bob, carol"
              onChange={event => onChange({ people: event.target.value })}
            />
          </Field>
          <Field label={t('triggers.calendar.rotate')} htmlFor={`${id}-rotate`}>
            <NativeSelect
              id={`${id}-rotate`}
              value={form.rotate}
              onChange={event =>
                onChange({ rotate: event.target.value as 'day' | 'week' })
              }
            >
              <NativeSelectOption value="week">
                {t('triggers.calendar.rotateWeek')}
              </NativeSelectOption>
              <NativeSelectOption value="day">
                {t('triggers.calendar.rotateDay')}
              </NativeSelectOption>
            </NativeSelect>
          </Field>
          <Field
            label={t('triggers.calendar.startDate')}
            htmlFor={`${id}-start`}
            className="sm:col-span-2"
            hint={t('triggers.calendar.startDateHint')}
          >
            <Input
              id={`${id}-start`}
              type="date"
              value={form.startDate}
              aria-invalid={
                (form.people.trim() !== '' && !form.startDate) || undefined
              }
              onChange={event => onChange({ startDate: event.target.value })}
            />
          </Field>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <Field
            label={t('triggers.calendar.backup')}
            htmlFor={`${id}-backup`}
            hint={t('triggers.calendar.backupHint')}
          >
            <Input
              id={`${id}-backup`}
              value={form.backup}
              placeholder={t('triggers.calendar.backupPlaceholder')}
              onChange={event => onChange({ backup: event.target.value })}
            />
          </Field>
          <Field
            label={t('triggers.calendar.backupAfter')}
            htmlFor={`${id}-after`}
            hint={t('triggers.calendar.backupAfterHint')}
          >
            <Input
              id={`${id}-after`}
              inputMode="numeric"
              value={form.backupAfterMinutes}
              placeholder="60"
              onChange={event =>
                onChange({ backupAfterMinutes: event.target.value })
              }
            />
          </Field>
        </div>

        <Collapsible>
          <CollapsibleTrigger className="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground [&[data-state=open]>svg]:rotate-90">
            <ChevronRight aria-hidden className="size-4 rtl:rotate-180" />
            {t('triggers.calendar.byDay')}
          </CollapsibleTrigger>
          <CollapsibleContent className="grid gap-3 pt-3 sm:grid-cols-4">
            {WEEKDAYS.map(day => (
              <Field
                key={day}
                label={t(`triggers.calendar.weekday.${day}`)}
                htmlFor={`${id}-by-${day}`}
              >
                <Input
                  id={`${id}-by-${day}`}
                  value={form.byDay[day] ?? ''}
                  onChange={event =>
                    onChange({
                      byDay: { ...form.byDay, [day]: event.target.value },
                    })
                  }
                />
              </Field>
            ))}
            <p className="text-xs text-muted-foreground sm:col-span-4">
              {t('triggers.calendar.byDayHint')}
            </p>
          </CollapsibleContent>
        </Collapsible>

        <div className="flex flex-col gap-2">
          <Label>{t('triggers.calendar.cover')}</Label>
          {form.cover.map((row, index) => (
            <div key={index} className="flex flex-wrap gap-2">
              <Input
                aria-label={t('triggers.calendar.coverPerson')}
                className="w-36"
                value={row.person}
                placeholder={t('triggers.calendar.coverPerson')}
                onChange={event =>
                  setCover(index, { person: event.target.value })
                }
              />
              <Input
                aria-label={t('triggers.calendar.coverFrom')}
                type="date"
                className="w-40"
                value={row.from}
                onChange={event =>
                  setCover(index, { from: event.target.value })
                }
              />
              <Input
                aria-label={t('triggers.calendar.coverTo')}
                type="date"
                className="w-40"
                value={row.to}
                onChange={event => setCover(index, { to: event.target.value })}
              />
              <Input
                aria-label={t('triggers.calendar.coverBackup')}
                className="w-36"
                value={row.backup}
                placeholder={t('triggers.calendar.coverBackup')}
                onChange={event =>
                  setCover(index, { backup: event.target.value })
                }
              />
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label={t('triggers.calendar.removeCover')}
                onClick={() =>
                  onChange({ cover: form.cover.filter((_, i) => i !== index) })
                }
              >
                <X />
              </Button>
            </div>
          ))}
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="self-start"
            onClick={() => onChange({ cover: [...form.cover, EMPTY_COVER] })}
          >
            <Plus />
            {t('triggers.calendar.addCover')}
          </Button>
          <p className="text-xs text-muted-foreground">
            {t('triggers.calendar.coverHint')}
          </p>
        </div>
      </section>

      <CalendarPreviewList form={form} />
    </div>
  )
}

/** The next runs and who is on duty, from the scheduler's own rules */
function CalendarPreviewList({ form }: { form: TriggerFormState }) {
  const { t, i18n } = useTranslation()
  const preview = useCalendarPreview(calendarConfig(form))
  const data = preview.data

  if (!data) return null
  if (!data.valid) {
    return (
      <p role="alert" className="text-xs text-destructive">
        {t('triggers.calendar.invalid', { error: data.error ?? '' })}
      </p>
    )
  }
  if (data.next.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        {t('triggers.calendar.noRuns')}
      </p>
    )
  }
  return (
    <div aria-live="polite" className="flex flex-col gap-1">
      <Label>{t('triggers.calendar.nextRuns')}</Label>
      <ul className="flex flex-col gap-0.5 text-xs" data-testid="calendar-next">
        {data.next.map(run => (
          <li key={run.at} className="flex flex-wrap gap-x-2">
            <span className="tabular-nums text-muted-foreground">
              {new Date(run.at).toLocaleString(i18n.language, {
                weekday: 'short',
                day: 'numeric',
                month: 'short',
                hour: '2-digit',
                minute: '2-digit',
              })}
            </span>
            {run.person && (
              <span>
                {run.coveringFor
                  ? t('triggers.calendar.covering', {
                      person: run.person,
                      for: run.coveringFor,
                    })
                  : run.person}
              </span>
            )}
            {run.backup && (
              <span className="text-muted-foreground">
                {t('triggers.calendar.backupIs', { person: run.backup })}
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  )
}

function Field({
  label,
  htmlFor,
  hint,
  className,
  children,
}: {
  label: string
  htmlFor?: string
  hint?: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}
