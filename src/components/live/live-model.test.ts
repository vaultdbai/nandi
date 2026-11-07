import { describe, expect, it } from 'vitest'
import i18n from '@/i18n/config'
import type { LiveChange } from '@/lib/bindings'
import {
  authorText,
  changeFiles,
  changeTitle,
  updatedText,
  viewOf,
} from './live-model'

const t = i18n.getFixedT('en')

const change = (over: Partial<LiveChange> = {}): LiveChange => ({
  id: 'c1',
  area: 'notes',
  targetId: 'deadlines.md',
  label: 'deadlines.md',
  paths: ['deadlines.md'],
  pathCount: 1,
  action: 'updated',
  author: { kind: 'you' },
  at: 1_000_000,
  ...over,
})

describe('live model', () => {
  it('names who changed it', () => {
    expect(authorText({ kind: 'you' }, t)).toBe('by you')
    expect(
      authorText(
        {
          kind: 'agent',
          agentId: 'chief-of-staff',
          agentName: 'Chief of Staff',
        },
        t
      )
    ).toBe('by Chief of Staff')
    expect(
      authorText(
        {
          kind: 'workflow',
          workflowId: 'weekly',
          workflowName: 'Weekly report',
        },
        t
      )
    ).toBe('by the Weekly report workflow')
    expect(authorText({ kind: 'outside' }, t)).toBe('outside Nandi')
  })

  it('reads "Updated just now by …", then a relative time', () => {
    expect(updatedText(change(), 1_000_000 + 5_000, t, 'en')).toBe(
      'Updated just now by you'
    )
    expect(
      updatedText(
        change({ author: { kind: 'outside' } }),
        1_000_000 + 5 * 60_000,
        t,
        'en'
      )
    ).toBe('Updated 5 min. ago outside Nandi')
  })

  it('lists a work folder’s files, counting the rest', () => {
    expect(changeFiles(change(), t)).toBeNull()
    const work = change({
      area: 'work',
      targetId: 'writer',
      label: 'Writer',
      paths: ['a.md', 'b.md'],
      pathCount: 5,
    })
    expect(changeFiles(work, t)).toBe('a.md, b.md and 3 more')
    expect(changeFiles({ ...work, pathCount: 2 }, t)).toBe('a.md, b.md')
    expect(changeTitle(work, t)).toBe('Writer')
    expect(changeTitle(change({ label: '', area: 'notes' }), t)).toBe(
      'My notes'
    )
  })

  it('links each change to its page', () => {
    expect(viewOf(change())).toEqual({ kind: 'notes', note: 'deadlines.md' })
    expect(viewOf(change({ action: 'deleted' }))).toEqual({ kind: 'notes' })
    expect(viewOf(change({ area: 'work', targetId: 'writer' }))).toEqual({
      kind: 'agent',
      agentId: 'writer',
    })
    expect(viewOf(change({ area: 'workflow', targetId: 'weekly' }))).toEqual({
      kind: 'workflow',
      workflowId: 'weekly',
    })
    expect(viewOf(change({ area: 'policy', targetId: 'p' }))).toBeNull()
  })
})
