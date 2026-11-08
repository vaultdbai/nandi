/**
 * Network › Groups, without React: ordering, who may do what, and what
 * a person's access level means next to the group's policy.
 */
import type {
  AccessLevel,
  Approval,
  GroupSummary,
  GroupView,
} from '@/lib/bindings'

/** The five access levels, loosest first */
export const ACCESS_LEVELS: AccessLevel[] = [
  'fully-automated',
  'ask-first',
  'questions-only',
  'send-me-info',
  'none',
]

/** `fully-automated` → `fullyAutomated`, for i18n keys */
export function accessKey(level: AccessLevel): string {
  return level.replace(/-(\w)/g, (_, c: string) => c.toUpperCase())
}

/** My groups first, then by name */
export function sortGroups(groups: GroupSummary[]): GroupSummary[] {
  return [...groups].sort((a, b) => {
    const mine = Number(b.role !== null) - Number(a.role !== null)
    return mine || a.name.localeCompare(b.name)
  })
}

export function isAdmin(view: GroupView): boolean {
  return view.myRole === 'admin'
}

type Member = GroupView['group']['members'][number]

/** Admins on top, then by name (the instance ID when they have none) */
export function sortMembers(members: Member[]): Member[] {
  return [...members].sort((a, b) => {
    const admin = Number(b.role === 'admin') - Number(a.role === 'admin')
    return (
      admin ||
      (a.displayName || a.instanceId).localeCompare(
        b.displayName || b.instanceId
      )
    )
  })
}

/**
 * Whether the group's policy holds back what my level would allow: it
 * asks for every request (or a member's first) while I chose Fully
 * automated. `null` approval (an unreadable policy) refuses everything.
 */
export function policyIsStricter(
  approval: Approval | null,
  level: AccessLevel
): boolean {
  if (approval === null) return level !== 'none'
  return level === 'fully-automated' && approval !== 'per-member'
}

/** Whether removing or demoting `instanceId` would leave no admin */
export function isLastAdmin(view: GroupView, instanceId: string): boolean {
  const admins = view.group.members.filter(m => m.role === 'admin')
  return admins.length === 1 && admins[0]?.instanceId === instanceId
}

/** An invite's expiry `days` from now; `null`: until revoked */
export function expiresAt(
  days: number | null,
  now = Date.now()
): number | null {
  return days === null || days <= 0 ? null : now + days * 24 * 60 * 60 * 1000
}
