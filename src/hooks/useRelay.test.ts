import { describe, expect, it } from 'vitest'
import type { AgentCard } from '@/lib/bindings'
import { filterAgents, groupByCapability } from './useRelay'

function card(id: string, name: string, capabilities: string[]): AgentCard {
  return {
    id,
    name,
    description: '',
    instanceId: id.split('/')[0] ?? '',
    endpoint: '',
    capabilities: capabilities.map(capability => ({ name: capability })),
    acceptsDelegation: true,
    maxConcurrent: 1,
    currentLoad: 0,
    owner: { githubHandle: 'grace', displayName: 'Grace Hopper' },
    signature: null,
    issuedAt: 0,
  }
}

const papers = card('a/papers', 'Papers', ['search', 'summarize'])
const writer = card('b/writer', 'Writer', ['summarize', 'write'])

describe('groupByCapability', () => {
  it('lists each card under every capability it serves, sorted', () => {
    const groups = groupByCapability([writer, papers])
    expect(groups.map(g => g.capability)).toEqual([
      'search',
      'summarize',
      'write',
    ])
    expect(groups[1]?.cards.map(c => c.name)).toEqual(['Papers', 'Writer'])
  })

  it('ignores duplicate capability names on one card', () => {
    const twice = card('a/x', 'X', ['dup', 'dup'])
    expect(groupByCapability([twice])[0]?.cards).toHaveLength(1)
  })
})

describe('filterAgents', () => {
  it('matches name, capability, and owner, ignoring case', () => {
    expect(filterAgents([papers, writer], 'WRIT')).toEqual([writer])
    expect(filterAgents([papers, writer], 'search')).toEqual([papers])
    expect(filterAgents([papers, writer], 'hopper')).toHaveLength(2)
    expect(filterAgents([papers, writer], '  ')).toHaveLength(2)
    expect(filterAgents([papers, writer], 'zzz')).toEqual([])
  })
})
