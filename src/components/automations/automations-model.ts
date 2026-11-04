/**
 * Pure rules shared by the Automations pages: schedules in plain words,
 * priority levels, step labels, who started a run, workflow health, and
 * where a run or a firing opens. Tested without rendering.
 */
import cronstrue from 'cronstrue/i18n'
import type {
  Trigger,
  TriggerFire,
  WorkflowRun,
  WorkflowRunSource,
  WorkflowStepRun,
} from '@/lib/bindings'
import { sourceSummary } from '@/lib/trigger-form'
import type { View } from '@/store/navigation-store'

const DAY_MS = 24 * 60 * 60 * 1000

/** cronstrue's locale codes for ours (all four ship with it) */
const CRON_LOCALES: Record<string, string> = {
  en: 'en',
  fr: 'fr',
  ar: 'ar',
  ja: 'ja',
}

/**
 * A cron schedule in words ("At 09:00, Monday through Friday"), in the
 * UI language, or `null` when it doesn't parse
 */
export function describeSchedule(
  schedule: string,
  language: string
): string | null {
  const trimmed = schedule.trim()
  if (!trimmed) return null
  try {
    return cronstrue.toString(trimmed, {
      locale: CRON_LOCALES[language.split('-')[0] ?? 'en'] ?? 'en',
      use24HourTimeFormat: true,
      throwExceptionOnParseError: true,
      verbose: false,
    })
  } catch {
    return null
  }
}

/** Queue priorities behind the High / Normal / Low choice (lower runs sooner) */
export const PRIORITY_VALUES = { high: 30, normal: 80, low: 100 } as const
export type PriorityLevel = keyof typeof PRIORITY_VALUES | 'custom'

/**
 * The choice a stored priority shows as; anything else is "Custom". A
 * trigger left at its source type's default (`sourceDefault`) is Normal.
 */
export function priorityLevel(
  priority: number | null,
  sourceDefault?: number
): PriorityLevel {
  if (priority === null || priority === sourceDefault) return 'normal'
  const found = (
    Object.entries(PRIORITY_VALUES) as [keyof typeof PRIORITY_VALUES, number][]
  ).find(([, value]) => value === priority)
  return found ? found[0] : 'custom'
}

/** A step ID as words: `send-email` → `Send email` */
export function stepLabel(stepId: string): string {
  const words = stepId.replace(/[-_.]+/g, ' ').trim()
  if (!words) return stepId
  return words.charAt(0).toUpperCase() + words.slice(1)
}

export type RunStarter =
  | { kind: 'manual' }
  | { kind: 'trigger'; name: string | null; triggerId: string }
  | { kind: 'otherNandi' }
  | { kind: 'unknown' }

/** Who or what started a run, named for the "Started by" column */
export function runStarter(
  source: WorkflowRunSource | null,
  triggers: Trigger[] | undefined
): RunStarter {
  if (!source) return { kind: 'unknown' }
  switch (source.kind) {
    case 'manual':
      return { kind: 'manual' }
    case 'trigger':
      return {
        kind: 'trigger',
        triggerId: source.id,
        name: triggers?.find(t => t.id === source.id)?.name ?? null,
      }
    case 'gateway':
    case 'task':
      return { kind: 'otherNandi' }
  }
}

/**
 * Where a workflow run opens: its Request. In phase 1 a workflow run's
 * Request is its project view (`work-item`, same ID) until task 14.
 */
export function runRequestView(run: Pick<WorkflowRun, 'id'>): View {
  return { kind: 'work-item', id: run.id }
}

/** Where a firing that started work opens: the workflow run's Request, or the agent run */
export function fireView(
  fire: TriggerFire,
  trigger: Pick<Trigger, 'targetType'>
): View | null {
  if (!fire.runId) return null
  return trigger.targetType === 'workflow'
    ? runRequestView({ id: fire.runId })
    : { kind: 'run-detail', runId: fire.runId }
}

/**
 * Who a calendar firing was for, in words: "On duty: Dev", or for a backup
 * taking over "Lena took over from Dev: <why>". Null for other firings.
 */
export function dutyText(
  fire: Pick<TriggerFire, 'payload'>,
  t: (key: string, options?: Record<string, unknown>) => string
): string | null {
  const payload =
    fire.payload &&
    typeof fire.payload === 'object' &&
    !Array.isArray(fire.payload)
      ? (fire.payload as Record<string, unknown>)
      : {}
  const person =
    typeof payload.responsible === 'string' ? payload.responsible : null
  if (!person) return null
  if (payload.role === 'backup') {
    return t('triggers.duty.tookOver', {
      person,
      primary: typeof payload.primary === 'string' ? payload.primary : '',
      reason:
        typeof payload.handover_reason === 'string'
          ? payload.handover_reason
          : '',
    })
  }
  return t('triggers.duty.onDuty', { person })
}

export interface WorkflowHealth {
  total: number
  done: number
  failed: number
  /** Waiting on someone (approval, manual step) or paused */
  waiting: number
  /** Mean duration of finished runs */
  avgMs: number | null
}

/** Runs of the last `days` days, by outcome */
export function workflowHealth(
  runs: WorkflowRun[],
  now: number,
  days = 30
): WorkflowHealth {
  const recent = runs.filter(run => run.startedAt >= now - days * DAY_MS)
  const finished = recent.filter(run => run.endedAt !== null)
  const durations = finished.map(run => (run.endedAt ?? 0) - run.startedAt)
  return {
    total: recent.length,
    done: recent.filter(run => run.status === 'completed').length,
    failed: recent.filter(run => run.status === 'failed').length,
    waiting: recent.filter(run =>
      ['awaitingApproval', 'waiting', 'paused'].includes(run.status)
    ).length,
    avgMs:
      durations.length > 0
        ? durations.reduce((sum, d) => sum + d, 0) / durations.length
        : null,
  }
}

/** The triggers that start this workflow */
export function triggersFor(
  workflowId: string,
  triggers: Trigger[] | undefined
): Trigger[] {
  return (triggers ?? []).filter(
    trigger =>
      trigger.targetType === 'workflow' && trigger.targetId === workflowId
  )
}

/** When a trigger fires, in plain words (a schedule's cron sits in a tooltip) */
export function whenText(
  trigger: Trigger,
  t: (key: string, options?: Record<string, unknown>) => string,
  language: string
): { text: string; detail: string | null } {
  if (trigger.sourceType === 'cron') {
    const config = trigger.sourceConfig as { schedule?: unknown }
    const schedule = typeof config?.schedule === 'string' ? config.schedule : ''
    return {
      text: describeSchedule(schedule, language) ?? t('triggers.when.invalid'),
      detail: schedule,
    }
  }
  return {
    text: t(`triggers.when.${trigger.sourceType}`, {
      detail: sourceSummary(trigger),
    }),
    detail: null,
  }
}

/** What starts a workflow:"At 09:00, Monday through Friday · Webhook", or manual */
export function startedByText(
  triggers: Trigger[],
  t: (key: string) => string,
  language: string
): string {
  if (triggers.length === 0) return t('automations.manual')
  return triggers
    .map(trigger => {
      if (trigger.sourceType === 'cron') {
        const config = trigger.sourceConfig as { schedule?: unknown }
        const schedule =
          typeof config?.schedule === 'string' ? config.schedule : ''
        return describeSchedule(schedule, language) ?? trigger.name
      }
      return t(`triggers.source.${trigger.sourceType}`)
    })
    .join(' · ')
}

/** The step a run waits on the user for (approval or a manual step), oldest first */
export function waitingStep(
  run: WorkflowRun | undefined
): WorkflowStepRun | null {
  return (
    run?.steps.find(
      step =>
        step.status === 'awaitingApproval' || step.status === 'awaitingInput'
    ) ?? null
  )
}

/** The step a failed run stopped at */
export function failedStep(
  run: WorkflowRun | undefined
): WorkflowStepRun | null {
  if (run?.status !== 'failed') return null
  return (
    run.steps.find(
      step => step.status === 'failed' || step.status === 'rejected'
    ) ?? null
  )
}

/** 1-based position of the step a run is on, for "step N of M" */
export function currentStepNumber(run: WorkflowRun): number {
  const index = run.steps.findIndex(step =>
    ['running', 'awaitingApproval', 'awaitingInput', 'pending'].includes(
      step.status
    )
  )
  return index === -1 ? run.steps.length : index + 1
}

/** Whether a run is still going (Run is disabled meanwhile) */
export function isActive(run: WorkflowRun | undefined): boolean {
  return (
    run !== undefined &&
    ['running', 'awaitingApproval', 'waiting', 'paused'].includes(run.status)
  )
}

/** A sample request for a webhook trigger, ready to paste in a terminal */
export function webhookCurl(url: string): string {
  return [
    `curl -X POST ${url} \\`,
    `  -H "X-Trigger-Secret: <your secret>" \\`,
    `  -H "Content-Type: application/json" -d '{"test": true}'`,
  ].join('\n')
}
