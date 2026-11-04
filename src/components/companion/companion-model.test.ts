import { describe, expect, it } from 'vitest'
import type { InboxItem } from '@/components/inbox/inbox-model'
import {
  EXPIRY_WARNING_MS,
  handledOne,
  noticesFor,
  orbFor,
  poseFor,
  requestToPin,
  requestIdFromLink,
  summaryParts,
  type NoticeState,
} from './companion-model'

const NOW = 1_000_000

function approval(id: string, deadline: number | null = null): InboxItem {
  return {
    kind: 'approval',
    id,
    createdAt: NOW - 1000,
    deadline,
    agentName: 'Writer',
    approval: {
      id,
      agentId: 'writer',
      runId: 'r',
      conversationId: 'c',
      tool: 'shell',
      args: {},
      createdAt: NOW - 1000,
      expiresAt: deadline,
    },
  }
}

const failed: InboxItem = {
  kind: 'failed',
  id: 'failed-w1',
  createdAt: NOW,
  deadline: null,
  item: {} as never,
}

describe('requestToPin', () => {
  it('keeps a Request in the companion first, then opens it in Nandi', () => {
    const request = { kind: 'request', requestId: 'r1' } as const
    expect(requestToPin(request, null)).toBe('r1')
    expect(requestToPin(request, 'other')).toBe('r1')
    // Already pinned: its › opens the full page
    expect(requestToPin(request, 'r1')).toBeNull()
    expect(requestToPin({ kind: 'home' }, null)).toBeNull()
    expect(requestToPin({ kind: 'work-item', id: 'w1' }, null)).toBeNull()
  })
})

describe('poseFor', () => {
  it('waits, carries, rests, and is done for a moment', () => {
    expect(poseFor({ state: 'needsYou', badge: 2 }, true)).toBe('wait')
    expect(poseFor({ state: 'problem', badge: '!' }, false)).toBe('wait')
    expect(poseFor({ state: 'running', badge: null }, false)).toBe('carry')
    expect(poseFor({ state: 'running', badge: null }, true)).toBe('done')
    expect(poseFor({ state: 'quiet', badge: null }, false)).toBe('rest')
  })

  it('counts a handled item only when the Inbox shrinks', () => {
    expect(handledOne(null, 0)).toBe(false)
    expect(handledOne(2, 1)).toBe(true)
    expect(handledOne(1, 2)).toBe(false)
  })
})

describe('orbFor', () => {
  it('colours by what matters most', () => {
    expect(orbFor({ inboxCount: 2, running: 3, blocker: 'noModel' })).toEqual({
      state: 'problem',
      badge: '!',
    })
    expect(orbFor({ inboxCount: 2, running: 3, blocker: null })).toEqual({
      state: 'needsYou',
      badge: 2,
    })
    expect(orbFor({ inboxCount: 0, running: 3, blocker: null })).toEqual({
      state: 'running',
      badge: null,
    })
    expect(orbFor({ inboxCount: 0, running: 0, blocker: null })).toEqual({
      state: 'quiet',
      badge: null,
    })
  })
})

describe('summaryParts', () => {
  it('leaves out zeros, in a fixed order', () => {
    expect(summaryParts({ needsYou: 0, running: 3, doneToday: 5 })).toEqual([
      { key: 'running', count: 3 },
      { key: 'doneToday', count: 5 },
    ])
    expect(summaryParts({ needsYou: 0, running: 0, doneToday: 0 })).toEqual([])
  })
})

describe('noticesFor', () => {
  const look = (
    items: InboxItem[],
    state: NoticeState | null,
    opts: { away?: boolean; enabled?: boolean; now?: number } = {}
  ) =>
    noticesFor({
      items,
      state,
      now: opts.now ?? NOW,
      away: opts.away ?? true,
      enabled: opts.enabled ?? true,
    })

  it('notifies about a request that came back, unless that is turned off', () => {
    const returned: InboxItem = {
      kind: 'requestReturn',
      id: 'return-t1',
      createdAt: NOW,
      deadline: null,
      ret: {
        taskId: 't1',
        fromName: 'Lena',
        providerAgent: 'lena/gateway',
        capability: 'ask-me',
        outcome: 'answered',
        summary: 'Nov 12',
        request: 'When?',
        returnedAt: NOW,
      },
    }
    const start = look([], null).state
    const on = noticesFor({
      items: [returned],
      state: start,
      now: NOW,
      away: true,
      enabled: false,
      results: true,
    })
    expect(on.notices.map(n => n.kind)).toEqual(['result'])
    const off = noticesFor({
      items: [returned],
      state: start,
      now: NOW,
      away: true,
      enabled: true,
      results: false,
    })
    expect(off.notices).toEqual([])
  })

  it('never notifies about what was there at start', () => {
    const first = look([approval('a1')], null)
    expect(first.notices).toEqual([])
    expect(look([approval('a1')], first.state).notices).toEqual([])
  })

  it('notifies once about a new approval while away', () => {
    const start = look([], null).state
    const next = look([approval('a1')], start)
    expect(next.notices.map(n => [n.itemId, n.kind])).toEqual([
      ['a1', 'approval'],
    ])
    expect(look([approval('a1')], next.state).notices).toEqual([])
  })

  it("doesn't interrupt while the card is open, nor when turned off", () => {
    const start = look([], null).state
    const shown = look([approval('a1')], start, { away: false })
    expect(shown.notices).toEqual([])
    // Seen while shown: hiding later doesn't announce it after the fact
    expect(look([approval('a1')], shown.state).notices).toEqual([])
    expect(look([approval('a2')], start, { enabled: false }).notices).toEqual(
      []
    )
  })

  it('warns once, 5 minutes before an item expires', () => {
    const deadline = NOW + 30 * 60_000
    const start = look([approval('a1', deadline)], null).state
    const later = deadline - EXPIRY_WARNING_MS + 1000
    const warned = look([approval('a1', deadline)], start, { now: later })
    expect(warned.notices.map(n => n.kind)).toEqual(['expiring'])
    expect(
      look([approval('a1', deadline)], warned.state, { now: later + 1000 })
        .notices
    ).toEqual([])
  })

  it('a new item already close to expiring gets one notice, not two', () => {
    const start = look([], null).state
    const soon = approval('a1', NOW + 60_000)
    const first = look([soon], start)
    expect(first.notices.map(n => n.kind)).toEqual(['approval'])
    expect(look([soon], first.state).notices).toEqual([])
  })

  it('never notifies about failures', () => {
    const start = look([], null).state
    expect(look([failed], start).notices).toEqual([])
  })
})

describe('requestIdFromLink', () => {
  it('reads nandi://request/<id> links only', () => {
    expect(requestIdFromLink('nandi://request/task-1')).toBe('task-1')
    expect(requestIdFromLink('nandi://request/a%20b')).toBe('a b')
    expect(requestIdFromLink('nandi://request/')).toBeNull()
    expect(requestIdFromLink('https://example.com')).toBeNull()
    expect(requestIdFromLink(undefined)).toBeNull()
  })
})
