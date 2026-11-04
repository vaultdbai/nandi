import { describe, expect, it } from 'vitest'
import {
  fromServer,
  groupItems,
  parseQuery,
  scoreText,
  type PaletteItem,
} from './palette-items'
import { addRecent, clearRecents, loadRecents, MAX_RECENTS } from './recents'

const item = (kind: PaletteItem['kind'], name: string, score: number) =>
  fromServer({
    kind,
    id: name,
    name,
    description: '',
    score,
    args: null,
    navigate: null,
  })

describe('scoreText', () => {
  it('ranks exact > prefix > word > substring > description', () => {
    expect(scoreText('writer', 'Writer', '')).toBe(100)
    expect(scoreText('writ', 'Writer', '')).toBe(80)
    expect(scoreText('analyst', 'Portfolio Analyst', '')).toBe(60)
    expect(scoreText('folio', 'Portfolio Analyst', '')).toBe(40)
    expect(scoreText('markets', 'Risk', 'Watches markets')).toBe(20)
    expect(scoreText('cron', 'Triggers', '', ['cron'])).toBe(20)
    expect(scoreText('zzz', 'Writer', 'writes')).toBe(0)
  })
})

describe('parseQuery', () => {
  it('detects command mode', () => {
    expect(parseQuery('> pause')).toEqual({ commandMode: true, text: 'pause' })
    expect(parseQuery('  >')).toEqual({ commandMode: true, text: '' })
    expect(parseQuery(' writer ')).toEqual({
      commandMode: false,
      text: 'writer',
    })
  })
})

describe('groupItems', () => {
  it('groups by kind in display order, best first', () => {
    const groups = groupItems([
      item('tool', 't', 10),
      item('agent', 'a1', 20),
      item('action', 'x', 5),
      item('agent', 'a2', 80),
    ])
    expect(groups.map(g => g.id)).toEqual(['action', 'agent', 'tool'])
    expect(groups[1]?.items.map(i => i.name)).toEqual(['a2', 'a1'])
  })

  it('treats unknown server kinds as actions', () => {
    expect(item('mystery' as never, 'm', 1).kind).toBe('action')
  })
})

describe('recents', () => {
  it('keeps the newest first, deduplicated and bounded', () => {
    clearRecents()
    for (let i = 0; i < MAX_RECENTS + 2; i++)
      addRecent(item('agent', `a${i}`, 0))
    addRecent(item('agent', 'a3', 0))
    const recents = loadRecents()
    expect(recents).toHaveLength(MAX_RECENTS)
    expect(recents[0]?.name).toBe('a3')
    expect(recents.filter(r => r.name === 'a3')).toHaveLength(1)
  })

  it('ignores corrupt storage', () => {
    localStorage.setItem('nandi.palette.recents', '{not json')
    expect(loadRecents()).toEqual([])
    localStorage.setItem('nandi.palette.recents', '[1, {"key": 2}]')
    expect(loadRecents()).toEqual([])
    clearRecents()
  })
})
