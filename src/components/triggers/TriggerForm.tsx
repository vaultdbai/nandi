import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight, FolderOpen, Plus, X } from 'lucide-react'
import {
  describeSchedule,
  priorityLevel,
  PRIORITY_VALUES,
} from '@/components/automations/automations-model'
import { CalendarFields } from '@/components/triggers/CalendarFields'
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
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Textarea } from '@/components/ui/textarea'
import { useAgents } from '@/hooks/useAgents'
import { useWorkflows } from '@/hooks/useSnapshots'
import { useCronPreview } from '@/hooks/useTriggers'
import { isDesktop } from '@/lib/client'
import { logger } from '@/lib/logger'
import {
  CRON_PRESETS,
  DEFAULT_PRIORITIES,
  EVENT_SOURCES,
  FILE_EVENTS,
  type EventName,
  type TriggerFormState,
} from '@/lib/trigger-form'

interface TriggerFormProps {
  form: TriggerFormState
  onChange: (patch: Partial<TriggerFormState>) => void
}

/** The fields of a trigger, adapted to its source type. */
export function TriggerForm({ form, onChange }: TriggerFormProps) {
  const { t } = useTranslation()
  const id = useId()

  return (
    <div className="flex flex-col gap-5">
      <Row label={t('triggers.form.name')} htmlFor={`${id}-name`}>
        <Input
          id={`${id}-name`}
          value={form.name}
          placeholder={t('triggers.form.namePlaceholder')}
          onChange={event => onChange({ name: event.target.value })}
        />
      </Row>

      <SourceFields form={form} onChange={onChange} />

      <TargetFields form={form} onChange={onChange} />

      {/* Workflows take no input: their steps' messages apply */}
      {form.targetType === 'agent' && (
        <Row
          label={t('triggers.form.inputTemplate')}
          htmlFor={`${id}-template`}
          hint={t('triggers.form.inputTemplateHint')}
        >
          <Textarea
            id={`${id}-template`}
            rows={4}
            dir="auto"
            className="font-mono text-xs"
            value={form.inputTemplate}
            placeholder={t('triggers.form.inputTemplatePlaceholder')}
            onChange={event => onChange({ inputTemplate: event.target.value })}
          />
        </Row>
      )}

      <PriorityFields form={form} onChange={onChange} />

      <Collapsible>
        <CollapsibleTrigger className="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground [&[data-state=open]>svg]:rotate-90">
          <ChevronRight aria-hidden className="size-4 rtl:rotate-180" />
          {t('triggers.form.advanced')}
        </CollapsibleTrigger>
        <CollapsibleContent className="flex flex-col gap-4 pt-3">
          <Row
            label={t('triggers.form.rateLimit')}
            htmlFor={`${id}-rate`}
            hint={t('triggers.form.rateLimitHint')}
          >
            <Input
              id={`${id}-rate`}
              inputMode="numeric"
              value={form.rateLimitSeconds}
              placeholder="0"
              onChange={event =>
                onChange({ rateLimitSeconds: event.target.value })
              }
            />
          </Row>
          {priorityLevel(
            form.priority.trim() ? Number(form.priority) : null,
            DEFAULT_PRIORITIES[form.sourceType]
          ) === 'custom' && (
            <Row
              label={t('triggers.form.customPriority')}
              htmlFor={`${id}-priority`}
              hint={t('triggers.form.priorityHint')}
            >
              <Input
                id={`${id}-priority`}
                inputMode="numeric"
                value={form.priority}
                onChange={event => onChange({ priority: event.target.value })}
              />
            </Row>
          )}
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

/** High / Normal / Low; a number set by hand shows as Custom (Advanced) */
function PriorityFields({ form, onChange }: TriggerFormProps) {
  const { t } = useTranslation()
  const id = useId()
  const sourceDefault = DEFAULT_PRIORITIES[form.sourceType]
  const level = priorityLevel(
    form.priority.trim() ? Number(form.priority) : null,
    sourceDefault
  )

  return (
    <div className="flex flex-col gap-2">
      <Label id={`${id}-label`}>{t('triggers.form.priority')}</Label>
      <RadioGroup
        aria-labelledby={`${id}-label`}
        className="flex flex-wrap gap-5"
        value={level}
        onValueChange={value => {
          // Normal is the source's own default
          if (value === 'normal') onChange({ priority: '' })
          else if (value === 'high' || value === 'low')
            onChange({ priority: String(PRIORITY_VALUES[value]) })
        }}
      >
        {(['high', 'normal', 'low'] as const).map(choice => (
          <label key={choice} className="flex items-center gap-2 text-sm">
            <RadioGroupItem value={choice} />
            {t(`triggers.priority.${choice}`)}
          </label>
        ))}
        {level === 'custom' && (
          <label className="flex items-center gap-2 text-sm">
            <RadioGroupItem value="custom" />
            {t('triggers.priority.custom', { value: form.priority })}
          </label>
        )}
      </RadioGroup>
      <p className="text-xs text-muted-foreground">
        {t('triggers.priority.hint')}
      </p>
    </div>
  )
}

/**
 * A schedule: a preset or a custom cron expression, said in plain words,
 * with the next 3 runs from the scheduler's own parser
 */
function ScheduleFields({ form, onChange }: TriggerFormProps) {
  const { t, i18n } = useTranslation()
  const id = useId()
  const matched = CRON_PRESETS.find(p => p.schedule === form.schedule.trim())
  const [custom, setCustom] = useState(!matched)
  const preview = useCronPreview(form.schedule)
  const described = describeSchedule(form.schedule, i18n.language)
  // The scheduler's parser decides; the words are only a reading aid
  const invalid = !form.schedule.trim() || preview.data?.valid === false
  const next = preview.data?.valid ? preview.data.next : []

  return (
    <div className="flex flex-col gap-3">
      <Label id={`${id}-label`}>{t('triggers.form.when')}</Label>
      <RadioGroup
        aria-labelledby={`${id}-label`}
        className="flex flex-col gap-3"
        value={custom ? 'custom' : 'preset'}
        onValueChange={value => {
          setCustom(value === 'custom')
          if (value === 'preset' && !matched && CRON_PRESETS[0]) {
            onChange({ schedule: CRON_PRESETS[0].schedule })
          }
        }}
      >
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-sm">
            <RadioGroupItem value="preset" />
            {t('triggers.form.preset')}
          </label>
          <NativeSelect
            aria-label={t('triggers.form.cronPreset')}
            disabled={custom}
            value={matched?.id ?? ''}
            onChange={event => {
              const chosen = CRON_PRESETS.find(p => p.id === event.target.value)
              if (chosen) onChange({ schedule: chosen.schedule })
            }}
          >
            {!matched && <NativeSelectOption value="">—</NativeSelectOption>}
            {CRON_PRESETS.map(p => (
              <NativeSelectOption key={p.id} value={p.id}>
                {t(`triggers.cronPresets.${p.id}`)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-sm">
            <RadioGroupItem value="custom" />
            {t('triggers.form.custom')}
          </label>
          <Input
            id={`${id}-schedule`}
            aria-label={t('triggers.form.schedule')}
            aria-invalid={invalid || undefined}
            dir="ltr"
            className="max-w-56 font-mono"
            disabled={!custom}
            value={form.schedule}
            placeholder="0 9 * * 1-5"
            onChange={event => onChange({ schedule: event.target.value })}
          />
        </div>
      </RadioGroup>
      <div aria-live="polite" className="flex flex-col gap-0.5 text-xs">
        {invalid ? (
          <p role="alert" className="text-destructive">
            {t('triggers.form.invalidSchedule')}
          </p>
        ) : (
          <>
            {described && (
              <p className="text-sm text-foreground">
                {t('triggers.form.yourTime', { schedule: described })}
              </p>
            )}
            {next.length > 0 && (
              <p className="text-muted-foreground" data-testid="cron-next">
                {t('triggers.form.nextRuns', {
                  times: next
                    .map(at =>
                      new Date(at).toLocaleString(i18n.language, {
                        weekday: 'short',
                        day: 'numeric',
                        month: 'short',
                        hour: '2-digit',
                        minute: '2-digit',
                      })
                    )
                    .join(' · '),
                })}
              </p>
            )}
          </>
        )}
        <p className="text-muted-foreground">
          {t('triggers.form.scheduleHint')}
        </p>
      </div>
    </div>
  )
}

function Row({
  label,
  htmlFor,
  hint,
  children,
}: {
  label: string
  htmlFor?: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}

function SourceFields({ form, onChange }: TriggerFormProps) {
  const { t } = useTranslation()
  const id = useId()

  switch (form.sourceType) {
    case 'cron':
      return <ScheduleFields form={form} onChange={onChange} />
    case 'calendar':
      return <CalendarFields form={form} onChange={onChange} />
    case 'file':
      return (
        <div className="flex flex-col gap-4">
          <Row
            label={t('triggers.form.path')}
            htmlFor={`${id}-path`}
            hint={t('triggers.form.pathHint')}
          >
            <div className="flex gap-2">
              <Input
                id={`${id}-path`}
                dir="ltr"
                className="font-mono"
                value={form.path}
                placeholder="~/Downloads/contracts"
                onChange={event => onChange({ path: event.target.value })}
              />
              {isDesktop() && (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() =>
                    void pickFolder().then(path => path && onChange({ path }))
                  }
                >
                  <FolderOpen />
                  {t('triggers.form.browse')}
                </Button>
              )}
            </div>
          </Row>
          <div className="grid gap-4 sm:grid-cols-2">
            <Row
              label={t('triggers.form.patterns')}
              htmlFor={`${id}-patterns`}
              hint={t('triggers.form.patternsHint')}
            >
              <Input
                id={`${id}-patterns`}
                dir="ltr"
                className="font-mono"
                value={form.patterns}
                placeholder="*.pdf, *.docx"
                onChange={event => onChange({ patterns: event.target.value })}
              />
            </Row>
            <Row label={t('triggers.form.debounce')} htmlFor={`${id}-debounce`}>
              <Input
                id={`${id}-debounce`}
                inputMode="numeric"
                value={form.debounceMs}
                placeholder="2000"
                onChange={event => onChange({ debounceMs: event.target.value })}
              />
            </Row>
          </div>
          <fieldset className="flex flex-wrap items-center gap-x-5 gap-y-2">
            <legend className="mb-1.5 text-sm font-medium">
              {t('triggers.form.fileEvents')}
            </legend>
            {FILE_EVENTS.map(kind => (
              <label key={kind} className="flex items-center gap-2 text-sm">
                <Checkbox
                  checked={form.fileEvents.includes(kind)}
                  onCheckedChange={checked =>
                    onChange({
                      fileEvents:
                        checked === true
                          ? [...form.fileEvents, kind]
                          : form.fileEvents.filter(e => e !== kind),
                    })
                  }
                />
                {t(`triggers.fileEvents.${kind}`)}
              </label>
            ))}
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={form.recursive}
                onCheckedChange={checked =>
                  onChange({ recursive: checked === true })
                }
              />
              {t('triggers.form.recursive')}
            </label>
          </fieldset>
        </div>
      )
    case 'webhook':
      return (
        <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
          {t('triggers.form.webhookNote')}
        </p>
      )
    case 'device':
      return (
        <div className="grid gap-4 sm:grid-cols-2">
          <Row label={t('triggers.form.deviceId')} htmlFor={`${id}-device`}>
            <Input
              id={`${id}-device`}
              dir="ltr"
              className="font-mono"
              value={form.deviceId}
              placeholder="printer-01"
              onChange={event => onChange({ deviceId: event.target.value })}
            />
          </Row>
          <Row
            label={t('triggers.form.deviceEvents')}
            htmlFor={`${id}-device-events`}
            hint={t('triggers.form.deviceEventsHint')}
          >
            <Input
              id={`${id}-device-events`}
              dir="ltr"
              className="font-mono"
              value={form.deviceEvents}
              placeholder="paper_jam, ink_low"
              onChange={event => onChange({ deviceEvents: event.target.value })}
            />
          </Row>
        </div>
      )
    case 'event':
      return <EventFields form={form} onChange={onChange} />
  }
}

function EventFields({ form, onChange }: TriggerFormProps) {
  const { t } = useTranslation()
  const id = useId()
  const keys = EVENT_SOURCES[form.eventName]

  const setRow = (
    index: number,
    patch: Partial<{ key: string; value: string }>
  ) =>
    onChange({
      filter: form.filter.map((row, i) =>
        i === index ? { ...row, ...patch } : row
      ),
    })

  return (
    <div className="flex flex-col gap-4">
      <Row
        label={t('triggers.form.eventName')}
        htmlFor={`${id}-event`}
        hint={t(`triggers.events.${form.eventName}`)}
      >
        <NativeSelect
          id={`${id}-event`}
          value={form.eventName}
          onChange={event =>
            onChange({ eventName: event.target.value as EventName, filter: [] })
          }
        >
          {(Object.keys(EVENT_SOURCES) as EventName[]).map(name => (
            <NativeSelectOption key={name} value={name}>
              {name}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Row>
      <div className="flex flex-col gap-2">
        <Label>{t('triggers.form.filter')}</Label>
        {form.filter.map((row, index) => (
          <div key={index} className="flex gap-2">
            <NativeSelect
              aria-label={t('triggers.form.filterKey')}
              value={row.key}
              onChange={event => setRow(index, { key: event.target.value })}
            >
              <NativeSelectOption value="">—</NativeSelectOption>
              {keys.map(key => (
                <NativeSelectOption key={key} value={key}>
                  {key}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            <Input
              aria-label={t('triggers.form.filterValue')}
              dir="ltr"
              value={row.value}
              placeholder={row.key === 'status' ? 'failed' : ''}
              onChange={event => setRow(index, { value: event.target.value })}
            />
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={t('triggers.form.removeFilter')}
              onClick={() =>
                onChange({ filter: form.filter.filter((_, i) => i !== index) })
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
          onClick={() =>
            onChange({ filter: [...form.filter, { key: keys[0], value: '' }] })
          }
        >
          <Plus />
          {t('triggers.form.addFilter')}
        </Button>
        <p className="text-xs text-muted-foreground">
          {t('triggers.form.filterHint')}
        </p>
      </div>
    </div>
  )
}

function TargetFields({ form, onChange }: TriggerFormProps) {
  const { t } = useTranslation()
  const id = useId()
  const { data: agents } = useAgents()
  const { data: workflows } = useWorkflows()
  const options =
    form.targetType === 'agent'
      ? (agents ?? []).map(a => ({ id: a.id, name: a.name }))
      : (workflows ?? []).map(w => ({ id: w.id, name: w.name }))
  // Keep a saved target selectable even if it's gone from the list
  const known = !form.targetId || options.some(o => o.id === form.targetId)

  return (
    <div className="flex flex-col gap-2">
      <Label>{t('triggers.form.target')}</Label>
      <RadioGroup
        className="flex gap-5"
        value={form.targetType}
        onValueChange={value =>
          onChange({
            targetType: value as TriggerFormState['targetType'],
            targetId: '',
          })
        }
      >
        {(['agent', 'workflow'] as const).map(type => (
          <label key={type} className="flex items-center gap-2 text-sm">
            <RadioGroupItem value={type} id={`${id}-${type}`} />
            {t(`triggers.targetType.${type}`)}
          </label>
        ))}
      </RadioGroup>
      <NativeSelect
        aria-label={t('triggers.form.target')}
        value={form.targetId}
        onChange={event => onChange({ targetId: event.target.value })}
      >
        <NativeSelectOption value="">
          {t('triggers.form.chooseTarget')}
        </NativeSelectOption>
        {!known && (
          <NativeSelectOption value={form.targetId}>
            {form.targetId}
          </NativeSelectOption>
        )}
        {options.map(option => (
          <NativeSelectOption key={option.id} value={option.id}>
            {option.name}
          </NativeSelectOption>
        ))}
      </NativeSelect>
      {form.targetType === 'workflow' && (
        <p className="text-xs text-muted-foreground">
          {t('triggers.form.workflowNote')}
        </p>
      )}
    </div>
  )
}

/** The native folder picker (desktop only): the user chooses what's watched */
async function pickFolder(): Promise<string | null> {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const chosen = await open({ directory: true, multiple: false })
    return typeof chosen === 'string' ? chosen : null
  } catch (error) {
    logger.warn('Folder picker failed', { error: String(error) })
    return null
  }
}
