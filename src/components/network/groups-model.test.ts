import { describe, expect, it } from 'vitest'
import type { GroupSummary, GroupView } from '@/lib/bindings'
import {
  ACCESS_LEVELS,
  accessKey,
  expiresAt,
  isAdmin,
  isLastAdmin,
  policyIsStricter,
  sortGroups,
  sortMembers,
} from './groups-model'

const summary = (
  name: string,
  role: GroupSummary['role'] = null
): GroupSummary => ({
  id: `g_${name}`,
  name,
  description: '',
  memberCount: 1,
  createdAt: 0,
  policyVersion: 1,
  role,
})

const groupView = (
  members: [string, 'admin' | 'member'][],
  me = 'a'
): GroupView => ({
  group: {
    summary: summary('Finance', 'admin'),
    members: members.map(([instanceId, role]) => ({
      instanceId,
      displayName: instanceId.toUpperCase(),
      role,
      joinedAt: 0,
    })),
    policy: {
      version: 1,
      text: '',
      author: 'a',
      signature: '',
      updatedAt: 0,
    },
    invites: [],
  },
  policy: {
    verified: true,
    problem: null,
    version: 1,
    author: 'a',
    authorName: 'A',
    approval: 'always',
  },
  myRole: members.find(([id]) => id === me)?.[1] ?? null,
  myAccess: 'ask-first',
  me,
})

describe('groups model', () => {
  it('lists my groups first, then by name', () => {
    const sorted = sortGroups([
      summary('Zeta'),
      summary('Ops', 'member'),
      summary('Alpha'),
      summary('Finance', 'admin'),
    ])
    expect(sorted.map(g => g.name)).toEqual(['Finance', 'Ops', 'Alpha', 'Zeta'])
  })

  it('names access levels for i18n keys', () => {
    expect(ACCESS_LEVELS).toHaveLength(5)
    expect(accessKey('fully-automated')).toBe('fullyAutomated')
    expect(accessKey('send-me-info')).toBe('sendMeInfo')
    expect(accessKey('none')).toBe('none')
  })

  it('notes when the group policy is stricter than my level', () => {
    expect(policyIsStricter('always', 'fully-automated')).toBe(true)
    expect(policyIsStricter('first-time', 'fully-automated')).toBe(true)
    expect(policyIsStricter('per-member', 'fully-automated')).toBe(false)
    expect(policyIsStricter('always', 'ask-first')).toBe(false)
    expect(policyIsStricter(null, 'questions-only')).toBe(true)
    expect(policyIsStricter(null, 'none')).toBe(false)
  })

  it('keeps a group from losing its last admin', () => {
    const one = groupView([
      ['a', 'admin'],
      ['b', 'member'],
    ])
    expect(isAdmin(one)).toBe(true)
    expect(isLastAdmin(one, 'a')).toBe(true)
    expect(isLastAdmin(one, 'b')).toBe(false)
    const two = groupView([
      ['a', 'admin'],
      ['b', 'admin'],
    ])
    expect(isLastAdmin(two, 'a')).toBe(false)
    expect(
      sortMembers(
        groupView([
          ['b', 'member'],
          ['a', 'admin'],
        ]).group.members
      ).map(m => m.instanceId)
    ).toEqual(['a', 'b'])
  })

  it('turns days into an expiry', () => {
    expect(expiresAt(null)).toBeNull()
    expect(expiresAt(0)).toBeNull()
    expect(expiresAt(2, 1000)).toBe(1000 + 2 * 86_400_000)
  })
})
