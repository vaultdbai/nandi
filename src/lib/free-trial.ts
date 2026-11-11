/**
 * The free trial card's model: what to say about the tokens, and whom the
 * trial is linked to. Pure, so it's tested without the UI.
 */
import type { TrialAccount, TrialStatus } from '@/lib/bindings'

export interface TrialMessage {
  /** Under `models.trial.` */
  key:
    | 'usedUp'
    | 'paused'
    | 'unreachable'
    | 'checking'
    | 'active'
    | 'standby'
    | 'off'
  left?: number
  limit?: number
}

/**
 * What the card says. The gateway's count (`account`) wins over what the
 * app last heard (`trial`); `failed` is a failed `GET /v1/trial`.
 */
export function trialMessage(
  trial: TrialStatus,
  account: TrialAccount | undefined,
  failed: boolean
): TrialMessage {
  const left = account ? Math.max(account.limit - account.used, 0) : undefined
  const limit = account?.limit
  if (left === 0 || (!account && trial.usedUp)) return { key: 'usedUp', limit }
  if (account?.paused || (!account && trial.paused)) return { key: 'paused' }
  if (!account && (failed || trial.unreachable)) return { key: 'unreachable' }
  if (left === undefined) return { key: 'checking' }
  if (trial.active) return { key: 'active', left, limit }
  return { key: trial.enabled ? 'standby' : 'off', left, limit }
}

/** Whether the switch can turn the trial on */
export function trialUsedUp(
  trial: TrialStatus,
  account: TrialAccount | undefined
): boolean {
  return account ? account.used >= account.limit : trial.usedUp
}

/** Whom the trial is linked to: the connected email, or this computer */
export type TrialLinkLine =
  { key: 'linked'; email: string } | { key: 'perComputer' }

export function trialLinkLine(account: TrialAccount): TrialLinkLine {
  return account.account && account.email
    ? { key: 'linked', email: account.email }
    : { key: 'perComputer' }
}
