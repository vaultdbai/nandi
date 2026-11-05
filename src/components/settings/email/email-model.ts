/**
 * Settings › Email, the pure part: what each mailbox card says and offers,
 * how a sign-in problem is explained, and the watch list's text form.
 */
import type {
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
