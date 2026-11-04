/**
 * "Can {member} receive from me?" and "Can I receive from {member}?"
 * (network.md §4.3): one line per opt-in, from what the member's profile
 * and my own status already say. Pure, so every check is tested.
 *
 * Whether they trust me isn't published; it's read from how my last
 * request to them ended (refused for trust means no). Until I've asked,
 * it's unknown, not a failure.
 */
import type { MemberProfile, TaskRecord } from '@/lib/bindings'

export type CheckStatus = 'pass' | 'fail' | 'unknown'

export type SendCheckKey =
  'online' | 'listed' | 'general' | 'byName' | 'trustsMe' | 'notPaused'

export type ReceiveCheckKey = 'myReachable' | 'iTrust' | 'notPausedByMe'

export interface Check<K extends string> {
  key: K
  status: CheckStatus
}

/** My last request to them, if any (theirs to me don't count) */
export function lastRequestToThem(
  tasks: TaskRecord[],
  myInstance: string,
  theirInstance: string | null
): TaskRecord | null {
  if (!theirInstance) return null
  const mine = tasks.filter(
    task =>
      task.requesterInstance === myInstance &&
      task.providerInstance === theirInstance
  )
  mine.sort((a, b) => b.createdAt - a.createdAt)
  return mine[0] ?? null
}

/** Refused because they don't trust me (the provider's own words) */
export function refusedForTrust(task: TaskRecord): boolean {
  return task.state === 'rejected' && /trust/i.test(task.error ?? '')
}

export function canSendChecks(
  profile: MemberProfile,
  myInstance: string
): Check<SendCheckKey>[] {
  const gateway = profile.gateway
  const last = lastRequestToThem(
    profile.recentDelegations,
    myInstance,
    profile.instanceId
  )
  const trustsMe: CheckStatus =
    last === null ? 'unknown' : refusedForTrust(last) ? 'fail' : 'pass'
  const blocked =
    profile.trust.delegationsPaused || profile.trust.blockedAgents.includes('*')
  return [
    { key: 'online', status: profile.online ? 'pass' : 'fail' },
    { key: 'listed', status: gateway ? 'pass' : 'fail' },
    {
      key: 'general',
      status: gateway?.acceptsDelegation ? 'pass' : 'fail',
    },
    {
      key: 'byName',
      status:
        gateway?.directDelegation === true
          ? 'pass'
          : gateway?.directDelegation === false
            ? 'fail'
            : 'unknown',
    },
    { key: 'trustsMe', status: trustsMe },
    { key: 'notPaused', status: blocked ? 'fail' : 'pass' },
  ]
}

/**
 * `trustsRelayMembers`: "Trust everyone on this relay" is on and this
 * relay needs a token, so everyone it lists is trusted (RL-34)
 */
export function canReceiveChecks(
  profile: MemberProfile,
  myReachable: boolean,
  trustsRelayMembers = false
): Check<ReceiveCheckKey>[] {
  const blocked =
    profile.trust.delegationsPaused || profile.trust.blockedAgents.includes('*')
  return [
    { key: 'myReachable', status: myReachable ? 'pass' : 'fail' },
    // Not trusted isn't a failure: their requests wait for me
    {
      key: 'iTrust',
      status:
        profile.trust.addedAsTrustedPeer || trustsRelayMembers
          ? 'pass'
          : 'unknown',
    },
    { key: 'notPausedByMe', status: blocked ? 'fail' : 'pass' },
  ]
}

/** The answer line: yes when nothing fails */
export function verdict<K extends string>(checks: Check<K>[]): boolean {
  return checks.every(check => check.status !== 'fail')
}

/**
 * What they could do for my requests to get through, as steps of a plain
 * message (in order); checks they can't fix by being asked are left out
 */
export const ASKABLE: readonly SendCheckKey[] = [
  'listed',
  'general',
  'trustsMe',
]

export function askableSteps(checks: Check<SendCheckKey>[]): SendCheckKey[] {
  return checks
    .filter(check => check.status === 'fail' && ASKABLE.includes(check.key))
    .map(check => check.key)
}
