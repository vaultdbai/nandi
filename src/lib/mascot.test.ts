import { describe, expect, it } from 'vitest'
import { fnv1a, isMascotPhrase, nandiState, normalizePhrase } from './mascot'

describe('the hidden phrase', () => {
  it('matches both spellings in any case, punctuation ignored', () => {
    // Tests don't ship; the words appear only here
    expect(fnv1a('appa yep yep')).toBe(0x07fd06eb)
    expect(fnv1a('appa yip yip')).toBe(0x3a9b45e3)
    for (const typed of [
      'Appa yep yep',
      'APPA, YIP YIP!',
      '  appa yep   yep. ',
    ]) {
      expect(isMascotPhrase(typed)).toBe(true)
    }
  })

  it('matches nothing else', () => {
    for (const typed of ['appa', 'approve all', 'run writer', '']) {
      expect(isMascotPhrase(typed)).toBe(false)
    }
    expect(normalizePhrase('Hello,   World!')).toBe('hello world')
  })
})

describe('nandiState', () => {
  const base = { pendingApprovals: 0, runningAgents: 0, doneUntil: 0, now: 10 }
  it('waits on you before working, and rests otherwise', () => {
    expect(nandiState(base)).toBe('rest')
    expect(nandiState({ ...base, runningAgents: 2 })).toBe('carry')
    expect(nandiState({ ...base, runningAgents: 2, pendingApprovals: 1 })).toBe(
      'wait'
    )
    expect(nandiState({ ...base, pendingApprovals: 1, doneUntil: 20 })).toBe(
      'done'
    )
  })
})
