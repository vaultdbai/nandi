import { describe, expect, it } from 'vitest'
import { changeSummary, suggestedCommitMessage } from './change-summary'

describe('changeSummary', () => {
  it('counts lines added and removed', () => {
    expect(changeSummary('a\nb\nc', 'a\nB\nc\nd')).toEqual({
      added: 2,
      removed: 1,
    })
    expect(changeSummary('same', 'same')).toEqual({ added: 0, removed: 0 })
  })
})

describe('suggestedCommitMessage', () => {
  it('names what changed', () => {
    expect(suggestedCommitMessage({ kind: 'agent', id: 'writer' })).toBe(
      'Update the writer agent'
    )
    expect(suggestedCommitMessage({ kind: 'gateway' })).toBe('Update NANDI.md')
  })
})
