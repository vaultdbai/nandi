import { describe, expect, it } from 'vitest'
import type { ChiefOfStaffDigest, DigestItem, QueuedWork } from '@/lib/bindings'
import {
  digestLabelKey,
  digestRows,
  digestSummary,
  digestTitleKey,
  queueRows,
} from './digest-model'

const NOW = new Date('2026-11-07T12:00:00Z')

function item(overrides: Partial<DigestItem> = {}): DigestItem {
  return {
    requestId: 'r1',
    title: 'Pay the power bill',
    source: 'email',
    kind: 'started',
    status: 'running',
    text: null,
    at: NOW.getTime() - 2 * 60 * 60 * 1000,
    ...overrides,
  }
}

const digest = (
  items: DigestItem[],
  since: number | null = 1,
  queue: QueuedWork[] = []
): ChiefOfStaffDigest => ({
  since,
  items,
  queue,
})

function queued(overrides: Partial<QueuedWork> = {}): QueuedWork {
  return {
    id: 'q1',
    at: NOW.getTime() - 2 * 60 * 60 * 1000,
    source: 'email',
    action: 'start',
    requestId: null,
    requestTitle: null,
    text: 'Your power bill is due Friday',
    sender: 'bills@power.example',
    screening: { verdict: 'clean', reasons: [], links: [] },
    status: 'pending',
    resultRequestId: null,
    ...overrides,
  }
}

describe('digest-model', () => {
  it('titles by whether there is a last look', () => {
    expect(digestTitleKey(digest([], 5))).toBe('home.chiefOfStaffDigest.title')
    expect(digestTitleKey(digest([], null))).toBe(
      'home.chiefOfStaffDigest.titleAlways'
    )
  })

  it('labels finished items by their end status', () => {
    const finished = (status: DigestItem['status']) =>
      digestLabelKey(item({ kind: 'finished', status }))
    expect(finished('done')).toBe('home.chiefOfStaffDigest.finished.done')
    expect(finished('failed')).toBe('home.chiefOfStaffDigest.finished.failed')
    expect(finished('cancelled')).toBe(
      'home.chiefOfStaffDigest.finished.cancelled'
    )
    expect(digestLabelKey(item({ kind: 'needsYou' }))).toBe(
      'home.chiefOfStaffDigest.kind.needsYou'
    )
  })

  it('builds rows with source, time and the news for updated only', () => {
    const rows = digestRows(
      digest([
        item({ kind: 'updated', text: 'Due Monday', source: 'calendar' }),
        item({ requestId: 'r2', kind: 'started', text: 'ignored' }),
      ]),
      'en',
      NOW
    )
    expect(rows[0]).toMatchObject({
      requestId: 'r1',
      text: 'Due Monday',
      sourceKey: 'home.chiefOfStaffDigest.source.calendar',
      when: '2 hr. ago',
    })
    expect(rows[1]?.text).toBeNull()
    expect(rows[1]?.sourceKey).toBe('home.chiefOfStaffDigest.source.email')
  })

  it('counts kinds in header order and skips empty ones', () => {
    const parts = digestSummary(
      digest([
        item({ kind: 'started' }),
        item({ kind: 'needsYou' }),
        item({ kind: 'started' }),
        item({ kind: 'finished' }),
      ])
    )
    expect(parts).toEqual([
      { kind: 'needsYou', count: 1 },
      { kind: 'started', count: 2 },
      { kind: 'finished', count: 1 },
    ])
    expect(digestSummary(digest([]))).toEqual([])
  })

  it('puts the waiting count first in the summary', () => {
    const parts = digestSummary(
      digest([item()], 1, [queued(), queued({ id: 'q2' })])
    )
    expect(parts).toEqual([
      { kind: 'queued', count: 2 },
      { kind: 'started', count: 1 },
    ])
  })

  it('maps queue rows by action and verdict', () => {
    const rows = queueRows(
      digest([], 1, [
        queued(),
        queued({
          id: 'q2',
          action: 'update',
          requestId: 'r9',
          requestTitle: 'Dentist',
          screening: {
            verdict: 'suspicious',
            reasons: ['Asks for a password'],
            links: [],
          },
        }),
        queued({
          id: 'q3',
          source: 'calendar',
          screening: {
            verdict: 'blocked',
            reasons: ['Link to a private address'],
            links: ['http://10.0.0.7/login'],
          },
        }),
      ]),
      'en',
      NOW
    )
    expect(rows[0]).toMatchObject({
      action: 'start',
      verdict: 'clean',
      reasons: [],
      canApprove: true,
      needsConfirm: false,
      sourceKey: 'home.chiefOfStaffDigest.source.email',
      when: '2 hr. ago',
    })
    expect(rows[1]).toMatchObject({
      action: 'update',
      requestTitle: 'Dentist',
      reasons: ['Asks for a password'],
      canApprove: true,
      needsConfirm: true,
    })
    expect(rows[2]).toMatchObject({
      verdict: 'blocked',
      canApprove: false,
      needsConfirm: false,
      sourceKey: 'home.chiefOfStaffDigest.source.calendar',
    })
  })
})
