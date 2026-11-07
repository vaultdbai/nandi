/**
 * Settings › Email, the pure part: what each mailbox card says and offers,
 * how a sign-in problem is explained, and the watch list's text form.
 */
import type {
  BackfillEstimate,
  BackfillRange,
  BackfillStatus,
  EmailProviderStatus,
  MailProvider,
  SignInProblemKind,
} from '@/lib/tauri-bindings'

export const PROVIDER_NAMES: Record<MailProvider, string> = {
  outlook: 'Outlook',
  gmail: 'Gmail',
}

export type CardTone = 'muted' | 'ok' | 'warning' | 'error'

export interface MailboxCard {
  provider: MailProvider
  name: string
  /** i18n key of the status line */
  statusKey: string
  tone: CardTone
  /** "Connected as …" */
  account: string | null
  /** Connect (or Reconnect) is offered */
  canConnect: boolean
  /** Reconnect rather than Connect */
  reconnect: boolean
  canCheck: boolean
  canPause: boolean
  canResume: boolean
  canDisconnect: boolean
  error: string | null
}

export function mailboxCard(status: EmailProviderStatus): MailboxCard {
  const name = PROVIDER_NAMES[status.provider]
  const account = status.accountEmail
    ? status.accountName
      ? `${status.accountName} (${status.accountEmail})`
      : status.accountEmail
    : null
  const base = {
    provider: status.provider,
    name,
    account,
    canConnect: false,
    reconnect: false,
    canCheck: false,
    canPause: false,
    canResume: false,
    canDisconnect: status.connected,
    error: null as string | null,
  }
  switch (status.state) {
    case 'notConnected':
      return {
        ...base,
        statusKey: status.available
          ? 'email.status.notConnected'
          : 'email.status.unavailable',
        tone: 'muted',
        canConnect: status.available,
      }
    case 'ready':
      return {
        ...base,
        statusKey: 'email.status.ready',
        tone: 'ok',
        canCheck: true,
        canPause: true,
      }
    case 'paused':
      return {
        ...base,
        statusKey: 'email.status.paused',
        tone: 'muted',
        canResume: true,
      }
    case 'needsSignIn':
      return {
        ...base,
        statusKey: 'email.status.needsSignIn',
        tone: 'warning',
        canConnect: status.available,
        reconnect: true,
        error: status.error,
      }
    case 'error':
      return {
        ...base,
        statusKey: 'email.status.error',
        tone: 'error',
        canCheck: true,
        canPause: true,
        error: status.error,
      }
  }
}

/** The i18n key that explains a sign-in problem */
export function problemKey(kind: SignInProblemKind): string {
  return `email.problem.${kind}`
}

/** One sender or domain per line (or comma separated), trimmed */
export function parseWatchList(text: string): string[] {
  return text
    .split(/[\n,;]/)
    .map(entry => entry.trim())
    .filter(Boolean)
}

export function formatWatchList(watch: string[]): string {
  return watch.join('\n')
}

/** The check interval a new install starts with, in minutes */
export const DEFAULT_INTERVAL_MINUTES = 30

/** The intervals offered, plus the current one if it's another */
export function intervalOptions(current: number): number[] {
  const offered = [5, 15, 30, 60, 120]
  return offered.includes(current)
    ? offered
    : [...offered, current].sort((a, b) => a - b)
}

/** "30 minutes" or "2 hours" */
export function intervalLabel(
  t: (key: string, options: { count: number }) => string,
  minutes: number
): string {
  return minutes >= 60 && minutes % 60 === 0
    ? t('email.hours', { count: minutes / 60 })
    : t('email.minutes', { count: minutes })
}

/** What Home's email to-do offers */
export type EmailTodo =
  | { kind: 'none' }
  /** Nothing connected: connect one of these */
  | { kind: 'connect'; providers: MailProvider[] }
  /** One connected: the other, as a small option */
  | { kind: 'another'; providers: MailProvider[] }

export function emailTodo(providers: EmailProviderStatus[]): EmailTodo {
  const available = providers.filter(p => p.available && !p.connected)
  if (available.length === 0) return { kind: 'none' }
  const connected = providers.some(p => p.connected)
  return connected
    ? { kind: 'another', providers: available.map(p => p.provider) }
    : { kind: 'connect', providers: available.map(p => p.provider) }
}

/** The background read's choices, in the order offered */
export const BACKFILL_RANGES = [
  'threeMonths',
  'year',
  'everything',
  'off',
] as const satisfies readonly BackfillRange[]

export type BackfillLine =
  | { key: null }
  | {
      key: string
      when: number | null
      count: number
      canPause: boolean
      canResume: boolean
      canStop: boolean
    }

/** What a mailbox card says about its background read */
export function backfillLine(status: BackfillStatus): BackfillLine {
  const base = {
    when: status.backTo,
    count: status.read,
    canPause: false,
    canResume: false,
    canStop: false,
  }
  switch (status.phase) {
    case 'off':
      return { key: null }
    case 'waiting':
      return { ...base, key: 'email.backfill.waiting', canStop: true }
    case 'reading':
      return {
        ...base,
        key:
          status.backTo === null
            ? 'email.backfill.starting'
            : 'email.backfill.reading',
        canPause: true,
        canStop: true,
      }
    case 'paused':
      return {
        ...base,
        key: 'email.backfill.paused',
        canResume: true,
        canStop: true,
      }
    case 'done':
      return { ...base, key: 'email.backfill.done' }
  }
}

/** "about N emails, about M runs" over every connected mailbox */
export function estimateTotals(estimates: BackfillEstimate[]): {
  emails: number
  runs: number
} {
  return estimates.reduce(
    (sum, e) => ({ emails: sum.emails + e.emails, runs: sum.runs + e.runs }),
    { emails: 0, runs: 0 }
  )
}
