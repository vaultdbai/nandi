import { describe, it, expect } from 'vitest'
import en from '../../locales/en.json'
import ar from '../../locales/ar.json'
import fr from '../../locales/fr.json'
import ja from '../../locales/ja.json'
import type { RouteReason } from '@/lib/bindings'
import { reasonText, routeReasonsOf } from './route-reasons'

const day = (ms: number) => `day ${ms}`

const EVERY: RouteReason[] = [
  { code: 'named', name: 'Bob' },
  { code: 'pinned', name: 'Bob' },
  { code: 'covers', name: 'Bob', awayName: 'Alice', until: 5 },
  { code: 'covers', name: 'Bob', awayName: 'Alice' },
  { code: 'free', name: 'Bob' },
  { code: 'busy', name: 'Bob', load: 3, max: 3, waitMinutes: 20 },
  { code: 'busy', name: 'Bob', load: 3, max: 3, waitMinutes: null },
  { code: 'offline', name: 'Bob' },
  { code: 'away', name: 'Bob', until: 5 },
  { code: 'away', name: 'Bob' },
  { code: 'usually_quick', name: 'Bob' },
  { code: 'dependable', name: 'Bob' },
  { code: 'lighter_day', name: 'Bob' },
  { code: 'their_turn', name: 'Bob' },
  { code: 'trying_again', name: 'Bob' },
]

describe('route reasons', () => {
  it('word optional parts only when the relay sent them', () => {
    expect(
      reasonText(
        { code: 'busy', name: 'Alice', load: 3, max: 3, waitMinutes: 20 },
        day
      )
    ).toEqual({
      key: 'routing.reason.busyWait',
      values: { name: 'Alice', load: 3, max: 3, minutes: 20 },
    })
    expect(
      reasonText(
        { code: 'covers', name: 'Bob', awayName: 'Alice', until: 7 },
        day
      )
    ).toEqual({
      key: 'routing.reason.coversUntil',
      values: { name: 'Bob', away: 'Alice', until: 'day 7' },
    })
    expect(
      reasonText({ code: 'away', name: 'Bob', until: null }, day).key
    ).toBe('routing.reason.away')
    expect(reasonText({ code: 'lighter_day', name: 'Bob' }, day)).toEqual({
      key: 'routing.reason.lighter_day',
      values: { name: 'Bob' },
    })
  })

  it('has every key in every locale, with no numbers that read as a track record', () => {
    for (const locale of [en, ar, fr, ja] as Record<string, string>[]) {
      for (const reason of EVERY) {
        const { key, values } = reasonText(reason, day)
        const text = locale[key]
        expect(text, key).toBeTruthy()
        for (const name of Object.keys(values)) {
          expect(text, `${key} uses {{${name}}}`).toContain(`{{${name}}}`)
        }
        expect(text).not.toContain('%')
      }
    }
  })

  it('reads reasons from a route message, dropping unknown ones', () => {
    expect(
      routeReasonsOf({
        provider: 'Bob',
        reasons: [
          { code: 'free', name: 'Bob' },
          { code: 'telepathy', name: 'Bob' },
          'free',
        ],
      })
    ).toEqual([{ code: 'free', name: 'Bob' }])
    expect(routeReasonsOf(null)).toEqual([])
    expect(routeReasonsOf({ reasons: 'free' })).toEqual([])
  })
})
