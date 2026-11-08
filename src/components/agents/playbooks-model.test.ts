import { describe, expect, it } from 'vitest'
import {
  EMPTY_DRAFT,
  MAX_LINE_CHARS,
  MAX_PLAYBOOK_WORDS,
  draftIssues,
  draftOf,
  isValidName,
  keepSelection,
  playbookNameFrom,
  sameDraft,
  toInput,
  wordCount,
  type PlaybookDraft,
} from './playbooks-model'

const good: PlaybookDraft = {
  name: 'pay-a-bill',
  description: 'Pay a bill online',
  when: 'A bill is due',
  body: '1. Plan.\n2. Pay.',
}

describe('draftIssues', () => {
  it('accepts a complete draft', () => {
    expect(draftIssues(good, [], null)).toEqual([])
  })

  it('needs every field', () => {
    expect(draftIssues(EMPTY_DRAFT, [], null)).toEqual([
      'nameRequired',
      'descriptionRequired',
      'whenRequired',
      'bodyRequired',
    ])
  })

  it('checks the name like the core', () => {
    expect(draftIssues({ ...good, name: 'Pay Bill' }, [], null)).toEqual([
      'nameFormat',
    ])
    expect(draftIssues(good, ['pay-a-bill'], null)).toEqual(['nameTaken'])
    // An edit keeps its own name
    expect(draftIssues(good, ['pay-a-bill'], 'pay-a-bill')).toEqual([])
    // A rename can't take another's
    expect(draftIssues(good, ['pay-a-bill'], 'old-name')).toEqual(['nameTaken'])
  })

  it('keeps the index lines short and single', () => {
    expect(
      draftIssues(
        { ...good, description: 'x'.repeat(MAX_LINE_CHARS + 1) },
        [],
        null
      )
    ).toEqual(['lineTooLong'])
    expect(draftIssues({ ...good, when: 'a\nb' }, [], null)).toEqual([
      'oneLine',
    ])
  })

  it('caps the body at 900 words', () => {
    const fits = 'word '.repeat(MAX_PLAYBOOK_WORDS)
    expect(draftIssues({ ...good, body: fits }, [], null)).toEqual([])
    expect(draftIssues({ ...good, body: `${fits} more` }, [], null)).toEqual([
      'tooLong',
    ])
  })
})

describe('names', () => {
  it('knows kebab-case', () => {
    expect(isValidName('web-task')).toBe(true)
    expect(isValidName('step-2')).toBe(true)
    for (const bad of ['', 'Web', 'a_b', '-a', 'a-', 'a--b']) {
      expect(isValidName(bad)).toBe(false)
    }
  })

  it('makes one from a title', () => {
    expect(playbookNameFrom('Pay a bill!')).toBe('pay-a-bill')
    expect(playbookNameFrom('  Café  run ')).toBe('cafe-run')
    expect(playbookNameFrom('***')).toBe('')
  })
})

describe('drafts', () => {
  it('compares and trims', () => {
    expect(sameDraft(good, { ...good, body: `${good.body}\n` })).toBe(true)
    expect(sameDraft(good, { ...good, when: 'Later' })).toBe(false)
    expect(toInput({ ...good, name: ' pay-a-bill ' }).name).toBe('pay-a-bill')
    expect(draftOf({ ...good })).toEqual(good)
  })

  it('counts words', () => {
    expect(wordCount('')).toBe(0)
    expect(wordCount('  one two\nthree ')).toBe(3)
  })
})

describe('keepSelection', () => {
  it('moves off a removed playbook', () => {
    expect(
      keepSelection({ kind: 'existing', name: 'gone' }, ['a', 'b'])
    ).toEqual({ kind: 'existing', name: 'a' })
    expect(keepSelection({ kind: 'existing', name: 'gone' }, [])).toEqual({
      kind: 'none',
    })
  })

  it('opens the first one, and keeps a new draft', () => {
    expect(keepSelection({ kind: 'none' }, ['a'])).toEqual({
      kind: 'existing',
      name: 'a',
    })
    expect(keepSelection({ kind: 'new' }, ['a'])).toEqual({ kind: 'new' })
  })
})
