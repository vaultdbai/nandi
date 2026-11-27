import { describe, expect, it } from 'vitest'
import en from '../../../locales/en.json'
import fr from '../../../locales/fr.json'
import ja from '../../../locales/ja.json'
import ar from '../../../locales/ar.json'
import {
  EXAMPLES,
  pickExamples,
  shownIds,
  SHOWN,
  type ExampleContext,
  type ExampleKind,
  type ExampleNeed,
} from './examples-model'

const ALL: ExampleNeed[] = ['mail', 'relay', 'phone']
const context = (over: Partial<ExampleContext> = {}): ExampleContext => ({
  connected: new Set(ALL),
  used: new Set(),
  previous: [],
  ...over,
})

describe('pickExamples', () => {
  it('shows a few of different kinds, the same for one seed, another for the next', () => {
    const one = pickExamples(1, context())
    expect(one).toHaveLength(SHOWN)
    const kinds = one.map(c => (c.kind === 'ask' ? c.example.kind : c.need))
    expect(new Set(kinds).size).toBe(SHOWN)
    expect(pickExamples(1, context())).toEqual(one)
    expect(shownIds(pickExamples(2, context()))).not.toEqual(shownIds(one))
  })

  it('never shows the last set again back to back', () => {
    for (let seed = 0; seed < 50; seed++) {
      const first = shownIds(pickExamples(seed, context()))
      const next = shownIds(
        pickExamples(seed + 1, context({ previous: first }))
      )
      expect(next.filter(id => first.includes(id))).toEqual([])
    }
  })

  it('brings untried kinds forward and steps used ones back', () => {
    const used = new Set<ExampleKind>([
      'mail',
      'calendar',
      'browser',
      'automations',
      'agents',
    ])
    for (let seed = 0; seed < 30; seed++) {
      const chips = pickExamples(seed, context({ used }))
      for (const chip of chips) {
        expect(chip.kind === 'ask' && used.has(chip.example.kind)).toBe(false)
      }
    }
  })

  it('turns an ask that needs something missing into the step that connects it', () => {
    const nothing = context({ connected: new Set() })
    for (let seed = 0; seed < 30; seed++) {
      const chips = pickExamples(seed, nothing)
      for (const chip of chips) {
        if (chip.kind === 'ask') expect(chip.example.needs).toBeUndefined()
      }
      const connects = chips.filter(c => c.kind === 'connect')
      expect(new Set(connects.map(c => c.need)).size).toBe(connects.length)
    }
  })
})

describe('the pool', () => {
  const locales: Record<string, Record<string, string>> = { fr, ja, ar }
  const english = en as Record<string, string>
  const keys = [
    'examples.label',
    ...EXAMPLES.map(e => `examples.${e.id}`),
    ...ALL.map(need => `examples.connect.${need}`),
  ]

  it('has about 30 asks of every kind', () => {
    expect(EXAMPLES.length).toBeGreaterThanOrEqual(28)
    expect(new Set(EXAMPLES.map(e => e.kind)).size).toBe(9)
    expect(new Set(EXAMPLES.map(e => e.id)).size).toBe(EXAMPLES.length)
  })

  it.each(Object.keys(locales))(
    'is written in %s, not copied from English',
    lang => {
      const locale = locales[lang] ?? {}
      for (const key of keys) {
        expect(english[key], key).toBeTruthy()
        expect(locale[key], `${lang} ${key}`).toBeTruthy()
        expect(locale[key], `${lang} ${key}`).not.toBe(english[key])
      }
    }
  )
})
