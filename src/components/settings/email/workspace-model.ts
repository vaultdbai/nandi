/**
 * Settings › Calendar & files: what each account's card says and offers,
 * and the calendar check intervals. Pure.
 */

import type { AccountKind, WorkspaceAccountStatus } from '@/lib/bindings'
import type { CardTone } from './email-model'

/** Product names: not translated */
export const ACCOUNT_KIND_NAMES: Record<AccountKind, string> = {
  outlookCalendar: 'Outlook calendar',
  googleCalendar: 'Google Calendar',
  oneDrive: 'OneDrive',
  googleDrive: 'Google Drive',
}

export interface WorkspaceCard {
  name: string
  /** Under `workspace.state.` */
  statusKey:
    'notAvailable' | 'notConnected' | 'connected' | 'needsSignIn' | 'error'
  tone: CardTone
  account: string | null
  error: string | null
  canConnect: boolean
  reconnect: boolean
  canDisconnect: boolean
  canCheck: boolean
}

export function workspaceCard(status: WorkspaceAccountStatus): WorkspaceCard {
  const name = ACCOUNT_KIND_NAMES[status.kind]
  const base = {
    name,
    account: status.connected ? status.accountEmail : null,
    error: status.error,
    canDisconnect: status.connected,
    canCheck: status.connected && status.family === 'calendar',
  }
  if (!status.available) {
    return {
      ...base,
      statusKey: 'notAvailable',
      tone: 'muted',
      canConnect: false,
      reconnect: false,
      canDisconnect: false,
      canCheck: false,
    }
  }
  if (status.connected && status.needsSignIn) {
    return {
      ...base,
      statusKey: 'needsSignIn',
      tone: 'warning',
      canConnect: true,
      reconnect: true,
      canCheck: false,
    }
  }
  if (status.connected && status.error) {
    return {
      ...base,
      statusKey: 'error',
      tone: 'error',
      canConnect: true,
      reconnect: true,
    }
  }
  if (status.connected) {
    return {
      ...base,
      statusKey: 'connected',
      tone: 'ok',
      canConnect: false,
      reconnect: false,
    }
  }
  return {
    ...base,
    statusKey: 'notConnected',
    tone: 'muted',
    canConnect: true,
    reconnect: false,
  }
}

export const CALENDAR_INTERVALS = [15, 30, 60, 120, 240] as const

/** The offered intervals, with the current one if it's set elsewhere */
export function calendarIntervalOptions(current: number): number[] {
  const options: number[] = [...CALENDAR_INTERVALS]
  if (current > 0 && !options.includes(current)) options.push(current)
  return options.sort((a, b) => a - b)
}
