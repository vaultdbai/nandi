/**
 * The trigger editor's model: form state ⇄ `TriggerInput`, plus the
 * helpers the views share (endpoints, cron presets). Pure, so it's tested
 * without rendering. Rust validates everything again on save.
 */

import type {
  Trigger,
  TriggerInput,
  TriggerSourceType,
  TriggerTargetType,
} from '@/lib/tauri-bindings'

export const SOURCE_TYPES: TriggerSourceType[] = [
  'cron',
  'calendar',
  'file',
  'webhook',
  'device',
  'event',
]

/** Calendar weekdays, Monday first (the config's names) */
export const WEEKDAYS = [
  'mon',
  'tue',
  'wed',
  'thu',
  'fri',
  'sat',
  'sun',
] as const
export type Weekday = (typeof WEEKDAYS)[number]

/** Someone standing in on a date range (`YYYY-MM-DD`, both included) */
export interface CoverRow {
  person: string
  from: string
  to: string
  /** Empty: the calendar's backup */
  backup: string
}

export const FILE_EVENTS = ['create', 'modify', 'delete'] as const
export type FileEvent = (typeof FILE_EVENTS)[number]

/** Internal events an `event` trigger can watch, with filterable fields */
export const EVENT_SOURCES = {
  'agent.turn_done': ['agent_id', 'status', 'source_type', 'source_id'],
  'workflow.done': ['workflow_id'],
  'workflow.failed': ['workflow_id', 'step_id'],
} as const
export type EventName = keyof typeof EVENT_SOURCES

/** Ready-made schedules for the cron picker (POSIX, local time) */
export const CRON_PRESETS = [
  { id: 'everyFiveMinutes', schedule: '*/5 * * * *' },
  { id: 'hourly', schedule: '0 * * * *' },
  { id: 'dailyNine', schedule: '0 9 * * *' },
  { id: 'weekdaysNine', schedule: '0 9 * * 1-5' },
  { id: 'mondayNine', schedule: '0 9 * * 1' },
] as const

/** Default priorities by source, as Rust applies them (lower runs sooner) */
export const DEFAULT_PRIORITIES: Record<TriggerSourceType, number> = {
  cron: 80,
  calendar: 80,
  file: 70,
  webhook: 60,
  device: 50,
  event: 60,
}

export interface FilterRow {
  key: string
  value: string
}

export interface TriggerFormState {
  name: string
  enabled: boolean
  sourceType: TriggerSourceType
  // cron
  schedule: string
  // calendar
  /** Comma-separated `HH:MM` times */
  times: string
  /** Empty: every day */
  days: Weekday[]
  /** Comma-separated `YYYY-MM-DD` dates */
  skipDates: string
  /** Comma-separated names, taking turns */
  people: string
  rotate: 'day' | 'week'
  startDate: string
  byDay: Partial<Record<Weekday, string>>
  cover: CoverRow[]
  /** Empty: whoever's turn is next */
  backup: string
  backupAfterMinutes: string
  // file
  path: string
  /** Comma-separated globs, e.g. `*.pdf, *.docx` */
  patterns: string
  fileEvents: FileEvent[]
  debounceMs: string
  recursive: boolean
  // device
  deviceId: string
  /** Comma-separated event names; empty fires on all */
  deviceEvents: string
  // event
  eventName: EventName
  filter: FilterRow[]
  // target and behaviour
  targetType: TriggerTargetType
  targetId: string
  inputTemplate: string
  /** Empty: the source type's default */
  priority: string
  rateLimitSeconds: string
}

export function emptyForm(
  sourceType: TriggerSourceType = 'cron'
): TriggerFormState {
  return {
    name: '',
    enabled: true,
    sourceType,
    schedule: '0 9 * * 1-5',
    times: '09:00',
    days: ['mon', 'tue', 'wed', 'thu', 'fri'],
    skipDates: '',
    people: '',
    rotate: 'week',
    startDate: '',
    byDay: {},
    cover: [],
    backup: '',
    backupAfterMinutes: '',
    path: '',
    patterns: '',
    fileEvents: ['create', 'modify'],
    debounceMs: '',
    recursive: false,
    deviceId: '',
    deviceEvents: '',
    eventName: 'agent.turn_done',
    filter: [],
    targetType: 'agent',
    targetId: '',
    inputTemplate: '',
    priority: '',
    rateLimitSeconds: '',
  }
}

/** `"a, b,,c "` → `["a", "b", "c"]` */
export function parseList(text: string): string[] {
  return text
    .split(',')
    .map(part => part.trim())
    .filter(Boolean)
}

/** A whole number, or `null` for empty/invalid input */
function parseNumber(text: string): number | null {
  const trimmed = text.trim()
  if (!/^-?\d+$/.test(trimmed)) return null
  return Number(trimmed)
}

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}

const asString = (value: unknown) => (typeof value === 'string' ? value : '')

const asStrings = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter(v => typeof v === 'string') : []

/** The source config Rust expects for this form */
export function sourceConfig(form: TriggerFormState): Record<string, unknown> {
  switch (form.sourceType) {
    case 'cron':
      return { schedule: form.schedule.trim() }
    case 'calendar':
      return calendarConfig(form)
    case 'file': {
      const debounce = parseNumber(form.debounceMs)
      return {
        path: form.path.trim(),
        patterns: parseList(form.patterns),
        events: form.fileEvents,
        ...(debounce !== null && debounce >= 0 && { debounceMs: debounce }),
        recursive: form.recursive,
      }
    }
    case 'webhook':
      return {}
    case 'device':
      return {
        deviceId: form.deviceId.trim(),
        events: parseList(form.deviceEvents),
      }
    case 'event':
      return {
        eventName: form.eventName,
        filter: Object.fromEntries(
          form.filter
            .filter(row => row.key.trim() && row.value.trim())
            .map(row => [row.key.trim(), row.value.trim()])
        ),
      }
  }
}

/** The calendar config Rust expects, leaving out what isn't set */
export function calendarConfig(
  form: TriggerFormState
): Record<string, unknown> {
  const people = parseList(form.people)
  const byDay = Object.fromEntries(
    Object.entries(form.byDay)
      .map(([day, person]) => [day, (person ?? '').trim()])
      .filter(([, person]) => person)
  )
  const cover = form.cover
    .filter(row => row.person.trim() && row.from && row.to)
    .map(row => ({
      person: row.person.trim(),
      from: row.from,
      to: row.to,
      ...(row.backup.trim() && { backup: row.backup.trim() }),
    }))
  const minutes = parseNumber(form.backupAfterMinutes)
  return {
    times: parseList(form.times),
    ...(form.days.length > 0 && form.days.length < 7 && { days: form.days }),
    ...(parseList(form.skipDates).length > 0 && {
      skipDates: parseList(form.skipDates),
    }),
    ...(people.length > 0 && {
      people,
      rotate: form.rotate,
      startDate: form.startDate,
    }),
    ...(Object.keys(byDay).length > 0 && { byDay }),
    ...(cover.length > 0 && { cover }),
    ...(form.backup.trim() && { backup: form.backup.trim() }),
    ...(minutes !== null && minutes > 0 && { backupAfterMinutes: minutes }),
  }
}

export function toInput(form: TriggerFormState): TriggerInput {
  const priority = parseNumber(form.priority)
  const rateLimit = parseNumber(form.rateLimitSeconds)
  return {
    name: form.name.trim(),
    enabled: form.enabled,
    sourceType: form.sourceType,
    sourceConfig: sourceConfig(form) as TriggerInput['sourceConfig'],
    targetType: form.targetType,
    targetId: form.targetId,
    inputTemplate: form.inputTemplate.trim() ? form.inputTemplate : null,
    priority,
    rateLimitSeconds: rateLimit !== null && rateLimit > 0 ? rateLimit : null,
  }
}

/** The editor state for an existing trigger */
export function fromTrigger(trigger: Trigger): TriggerFormState {
  const config = asRecord(trigger.sourceConfig)
  const base = emptyForm(trigger.sourceType)
  const eventName = asString(config.eventName)
  const calendar = trigger.sourceType === 'calendar'
  return {
    ...base,
    name: trigger.name,
    enabled: trigger.enabled,
    schedule: asString(config.schedule) || base.schedule,
    ...(calendar && calendarFields(config)),
    path: asString(config.path),
    patterns: asStrings(config.patterns).join(', '),
    fileEvents:
      trigger.sourceType === 'file' && Array.isArray(config.events)
        ? (asStrings(config.events).filter(e =>
            (FILE_EVENTS as readonly string[]).includes(e)
          ) as FileEvent[])
        : base.fileEvents,
    debounceMs:
      typeof config.debounceMs === 'number' ? String(config.debounceMs) : '',
    recursive: config.recursive === true,
    deviceId: asString(config.deviceId),
    deviceEvents:
      trigger.sourceType === 'device'
        ? asStrings(config.events).join(', ')
        : '',
    eventName:
      eventName in EVENT_SOURCES ? (eventName as EventName) : base.eventName,
    filter: Object.entries(asRecord(config.filter)).map(([key, value]) => ({
      key,
      value: typeof value === 'string' ? value : JSON.stringify(value),
    })),
    targetType: trigger.targetType,
    targetId: trigger.targetId,
    inputTemplate: trigger.inputTemplate ?? '',
    priority: String(trigger.priority),
    rateLimitSeconds:
      trigger.rateLimitSeconds === null ? '' : String(trigger.rateLimitSeconds),
  }
}

/** A saved calendar config as form fields */
function calendarFields(
  config: Record<string, unknown>
): Partial<TriggerFormState> {
  const days = asStrings(config.days).filter((d): d is Weekday =>
    (WEEKDAYS as readonly string[]).includes(d)
  )
  const byDay = Object.fromEntries(
    Object.entries(asRecord(config.byDay)).filter(
      ([day, person]) =>
        (WEEKDAYS as readonly string[]).includes(day) &&
        typeof person === 'string'
    )
  ) as Partial<Record<Weekday, string>>
  const cover = Array.isArray(config.cover)
    ? config.cover.map(item => {
        const row = asRecord(item)
        return {
          person: asString(row.person),
          from: asString(row.from),
          to: asString(row.to),
          backup: asString(row.backup),
        }
      })
    : []
  return {
    times: asStrings(config.times).join(', '),
    days: days.length > 0 ? days : [...WEEKDAYS],
    skipDates: asStrings(config.skipDates).join(', '),
    people: asStrings(config.people).join(', '),
    rotate: config.rotate === 'day' ? 'day' : 'week',
    startDate: asString(config.startDate),
    byDay,
    cover,
    backup: asString(config.backup),
    backupAfterMinutes:
      typeof config.backupAfterMinutes === 'number'
        ? String(config.backupAfterMinutes)
        : '',
  }
}

/** What's missing before the form can be saved (Rust checks the rest) */
export function formProblems(form: TriggerFormState): string[] {
  const problems: string[] = []
  if (!form.name.trim()) problems.push('name')
  if (!form.targetId) problems.push('target')
  if (form.sourceType === 'cron' && !form.schedule.trim())
    problems.push('schedule')
  if (form.sourceType === 'calendar') {
    if (parseList(form.times).length === 0) problems.push('times')
    if (parseList(form.people).length > 0 && !form.startDate)
      problems.push('startDate')
  }
  if (form.sourceType === 'file' && !form.path.trim()) problems.push('path')
  if (form.sourceType === 'file' && form.fileEvents.length === 0)
    problems.push('fileEvents')
  if (form.sourceType === 'device' && !form.deviceId.trim())
    problems.push('deviceId')
  return problems
}

/** The receiver path for webhook and device triggers, else `null` */
export function endpointPath(trigger: Trigger): string | null {
  const encode = encodeURIComponent
  if (trigger.sourceType === 'webhook') {
    return `/api/triggers/webhook/${encode(trigger.id)}`
  }
  if (trigger.sourceType === 'device') {
    const deviceId = asString(asRecord(trigger.sourceConfig).deviceId)
    return deviceId ? `/api/triggers/device/${encode(deviceId)}` : null
  }
  return null
}

/** A one-line description of what the trigger watches */
export function sourceSummary(trigger: Trigger): string {
  const config = asRecord(trigger.sourceConfig)
  switch (trigger.sourceType) {
    case 'cron':
      return asString(config.schedule)
    case 'calendar': {
      const times = asStrings(config.times).join(', ')
      const people = [
        ...asStrings(config.people),
        ...Object.values(asRecord(config.byDay)).filter(
          (p): p is string => typeof p === 'string'
        ),
      ]
      const unique = [...new Set(people)]
      return unique.length > 0 ? `${times} · ${unique.join(', ')}` : times
    }
    case 'file': {
      const patterns = asStrings(config.patterns)
      const path = asString(config.path)
      return patterns.length > 0 ? `${path} (${patterns.join(', ')})` : path
    }
    case 'webhook':
      return endpointPath(trigger) ?? ''
    case 'device': {
      const events = asStrings(config.events)
      const device = asString(config.deviceId)
      return events.length > 0 ? `${device}: ${events.join(', ')}` : device
    }
    case 'event':
      return asString(config.eventName)
  }
}
