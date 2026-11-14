import { useQuery } from '@tanstack/react-query'
import { useAgents } from '@/hooks/useAgents'
import { useWorkflows } from '@/hooks/useSnapshots'
import { client } from '@/lib/client'
import type { JsonValue, Trigger } from '@/lib/tauri-bindings'
import {
  calendarConfig,
  formProblems,
  type TriggerFormState,
} from '@/lib/trigger-form'
import { useClientQuery } from '@/hooks/useClientQuery'

/** The display name of what a trigger runs (its ID while unknown) */
export function useTargetName(
  trigger: Pick<Trigger, 'targetType' | 'targetId'>
): string {
  const { data: agents } = useAgents()
  const { data: workflows } = useWorkflows()
  const list =
    trigger.targetType === 'agent'
      ? (agents ?? []).map(a => ({ id: a.id, name: a.name }))
      : (workflows ?? []).map(w => ({ id: w.id, name: w.name }))
  return (
    list.find(item => item.id === trigger.targetId)?.name ?? trigger.targetId
  )
}

/**
 * Trigger queries. `trigger://*` events invalidate them (see `agent-sync`),
 * so views update live without polling.
 */
export function useTriggers() {
  return useClientQuery(['triggers'], () => client.listTriggers())
}

export function useTrigger(triggerId: string) {
  return useClientQuery(
    ['trigger', triggerId],
    () => client.getTrigger(triggerId),
    { staleTime: 0 }
  )
}

/** Newest first */
export function useTriggerFires(triggerId: string, limit = 50) {
  return useClientQuery(
    ['triggerFires', triggerId],
    () => client.listTriggerFires(triggerId, limit),
    { staleTime: 0 }
  )
}

/**
 * Whether `schedule` parses and its next 3 runs, checked by the
 * scheduler's own parser as the user types (idle while empty)
 */
export function useCronPreview(schedule: string) {
  const trimmed = schedule.trim()
  return useClientQuery(
    ['cronPreview', trimmed],
    () => client.previewCron(trimmed),
    {
      enabled: trimmed.length > 0,
      placeholderData: previous => previous,
      staleTime: 60_000,
    }
  )
}

/**
 * Whether a calendar config is valid and its next runs with who is on
 * duty, from the scheduler's own rules (idle while `config` is null)
 */
export function useCalendarPreview(config: Record<string, unknown> | null) {
  const key = config ? JSON.stringify(config) : ''
  return useClientQuery(
    ['calendarPreview', key],
    () => client.previewCalendar((config ?? {}) as JsonValue),
    {
      enabled: config !== null,
      placeholderData: previous => previous,
      staleTime: 60_000,
    }
  )
}

/**
 * What blocks saving `form`: the model's checks plus a cron schedule the
 * scheduler's parser rejects (so mistakes are caught before Save)
 */
export function useTriggerFormProblems(form: TriggerFormState): string[] {
  const preview = useCronPreview(
    form.sourceType === 'cron' ? form.schedule : ''
  )
  const calendar = useCalendarPreview(
    form.sourceType === 'calendar' ? calendarConfig(form) : null
  )
  const problems = formProblems(form)
  if (
    form.sourceType === 'calendar' &&
    calendar.data?.valid === false &&
    !problems.includes('calendar')
  ) {
    problems.push('calendar')
  }
  if (
    form.sourceType === 'cron' &&
    preview.data?.valid === false &&
    !problems.includes('schedule')
  ) {
    problems.push('schedule')
  }
  return problems
}

/** Base URL for webhook/device endpoints */
export function useWebOrigin() {
  return useQuery({
    queryKey: ['webOrigin'],
    queryFn: () => client.getWebOrigin(),
    staleTime: Infinity,
  })
}
