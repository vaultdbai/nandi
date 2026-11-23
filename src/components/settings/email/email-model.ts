/**
 * Settings › Email, the pure part: what each mailbox card says and offers,
 * how a sign-in problem is explained, and the watch list's text form.
 */
import type {
  BackfillEstimate,
  BackfillRange,
  BackfillStatus,
  EmailProviderStatus,
  MailboxId,
  MailboxStatus,
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
  /** "Name (address)", or the address alone */
  account: string
  /** Sign in again is offered */
  reconnect: boolean
  canPause: boolean
  canResume: boolean
  error: string | null
}

/** The mailbox a row's Pause, Sign out and older-mail controls act on */
export function mailboxOf(status: MailboxStatus): MailboxId {
  return { provider: status.provider, email: status.email }
}

/** "Name (address)", or the address alone */
export function accountLabel(
  status: Pick<MailboxStatus, 'name' | 'email'>
): string {
  return status.name ? `${status.name} (${status.email})` : status.email
}

/** The i18n key of what a provider with no mailbox says */
export function providerStatusKey(status: EmailProviderStatus): string {
  return status.available
    ? 'email.status.notConnected'
    : 'email.status.unavailable'
}

/** The providers a mailbox can still be added from (this build has them) */
export function addableProviders(
  providers: readonly EmailProviderStatus[]
): MailProvider[] {
  return providers.filter(p => p.available).map(p => p.provider)
}

export function mailboxCard(status: MailboxStatus): MailboxCard {
  const base = {
    provider: status.provider,
    name: PROVIDER_NAMES[status.provider],
    account: accountLabel(status),
    // A calendar whose sign-in ended reconnects with the account
    reconnect: status.calendar?.needsSignIn === true,
    canPause: false,
    canResume: false,
    error: null as string | null,
  }
  // Files and calendar only (a 0.2 drive or calendar without its mailbox)
  if (!status.reads.mail) {
    return { ...base, statusKey: 'email.status.noMail', tone: 'muted' }
  }
  switch (status.state) {
    case 'ready':
      return {
        ...base,
        statusKey: 'email.status.ready',
        tone: 'ok',
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
        reconnect: true,
        error: status.error,
      }
    case 'error':
      return {
        ...base,
        statusKey: 'email.status.error',
        tone: 'error',
        canPause: true,
        error: status.error,
      }
  }
}

/** Check now and Read again need a mailbox that is being read */
export function canCheckNow(mailboxes: readonly MailboxStatus[]): boolean {
  return mailboxes.some(
    m => m.reads.mail && (m.state === 'ready' || m.state === 'error')
  )
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
