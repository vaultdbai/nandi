import { describe, expect, it } from 'vitest'
import {
  changedLines,
  lineDiff,
  parsePlaybookProposal,
  PROPOSE_PLAYBOOK_TOOL,
} from './playbook-proposal-model'

const args = {
  name: 'pay-bills',
  description: 'Pay the household bills',
  when: 'A bill is due',
  body: '1. Open the site.\n2. Pay from Chase, never the card.',
  reason: 'You told me how you pay bills',
}

describe('parsePlaybookProposal', () => {
  it('reads a new playbook', () => {
    expect(parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, args)).toEqual({
      ...args,
      current: null,
    })
  })

  it('reads the playbook it replaces', () => {
    const proposal = parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, {
      ...args,
      current: { description: 'Pay bills', when: 'A bill is due', body: 'x' },
    })
    expect(proposal?.current).toEqual({
      description: 'Pay bills',
      when: 'A bill is due',
      body: 'x',
    })
    expect(proposal && changedLines(proposal)).toEqual(['description'])
  })

  it('ignores other tools and broken arguments', () => {
    expect(parsePlaybookProposal('write_file', args)).toBeNull()
    expect(parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, null)).toBeNull()
    expect(parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, [])).toBeNull()
    expect(
      parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, { ...args, name: ' ' })
    ).toBeNull()
    expect(
      parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, { name: 'x', body: 3 })
    ).toEqual({
      name: 'x',
      reason: '',
      description: '',
      when: '',
      body: '',
      current: null,
    })
  })

  it('a new playbook changes no lines', () => {
    const proposal = parsePlaybookProposal(PROPOSE_PLAYBOOK_TOOL, args)
    expect(proposal && changedLines(proposal)).toEqual([])
  })
})

describe('lineDiff', () => {
  it('keeps, removes and adds lines in reading order', () => {
    expect(lineDiff('a\nb\nc', 'a\nB\nc\nd')).toEqual([
      { kind: 'same', text: 'a' },
      { kind: 'removed', text: 'b' },
      { kind: 'added', text: 'B' },
      { kind: 'same', text: 'c' },
      { kind: 'added', text: 'd' },
    ])
  })

  it('handles empty sides', () => {
    expect(lineDiff('', 'a')).toEqual([{ kind: 'added', text: 'a' }])
    expect(lineDiff('a', '')).toEqual([{ kind: 'removed', text: 'a' }])
    expect(lineDiff('', '')).toEqual([])
  })

  it('shows very long bodies whole instead of diffing them', () => {
    const long = Array.from({ length: 401 }, (_, i) => `line ${i}`).join('\n')
    const diff = lineDiff(long, 'new')
    expect(diff.filter(line => line.kind === 'removed')).toHaveLength(401)
    expect(diff.at(-1)).toEqual({ kind: 'added', text: 'new' })
  })
})
