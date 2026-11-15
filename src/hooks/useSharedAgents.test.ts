import { describe, expect, it } from 'vitest'
import type { CapabilityProviders } from '@/lib/bindings'
import {
  autoProvider,
  filterCapabilities,
  filterSharedAgents,
  parseCallInput,
  providerCounts,
} from './useSharedAgents'
import {
  agentCard,
  providerOption,
  sharedAgent,
} from './useSharedAgents.fixtures'

const papers = sharedAgent(
  agentCard('alice', 'papers', {
    name: 'Papers',
    description: 'Finds research papers',
  })
)
const atlas = sharedAgent(
  agentCard('bob', 'atlas', {
    name: 'Atlas',
    capabilities: [{ name: 'geocode' }],
  }),
  { sources: ['relay'] }
)
const scribe = sharedAgent(agentCard('carol', 'scribe', { name: 'Scribe' }))
const all = [papers, atlas, scribe]
const names = (list: typeof all) => list.map(a => a.card.name)

describe('filterSharedAgents', () => {
  it('keeps everything with no search', () => {
    expect(names(filterSharedAgents(all, '  '))).toEqual([
      'Papers',
      'Atlas',
      'Scribe',
    ])
  })

  it('searches name, description, owner, and capability, ignoring case', () => {
    const search = (text: string) => names(filterSharedAgents(all, text))
    expect(search('  ATLAS ')).toEqual(['Atlas'])
    expect(search('research')).toEqual(['Papers'])
    expect(search('bob-owner')).toEqual(['Atlas'])
    expect(search('geocode')).toEqual(['Atlas'])
    expect(search('find-papers')).toEqual(['Papers', 'Scribe'])
    expect(search('nothing-like-it')).toEqual([])
  })
})

describe('parseCallInput', () => {
  it('sends JSON when it parses, else the text, and null when empty', () => {
    expect(parseCallInput('{"q": "llm"}')).toEqual({ q: 'llm' })
    expect(parseCallInput('find llm papers')).toBe('find llm papers')
    expect(parseCallInput('   ')).toBeNull()
  })
})

describe('capability helpers', () => {
  const entry: CapabilityProviders = {
    capability: 'find-papers',
    providers: [
      providerOption(atlas.card, { rank: 1 }),
      providerOption(papers.card),
    ],
    pinned: null,
    autoChoice: 'bob/atlas',
  }

  it('counts providers per capability', () => {
    expect(providerCounts([entry]).get('find-papers')).toBe(2)
    expect(providerCounts(undefined).size).toBe(0)
  })

  it('finds the provider Auto would use', () => {
    expect(autoProvider(entry)?.card.id).toBe('bob/atlas')
    expect(autoProvider({ ...entry, autoChoice: null })).toBeUndefined()
    expect(autoProvider(undefined)).toBeUndefined()
  })

  it('filters by capability, provider, or owner', () => {
    expect(filterCapabilities([entry], 'PAPERS')).toHaveLength(1)
    expect(filterCapabilities([entry], 'bob-owner')).toHaveLength(1)
    expect(filterCapabilities([entry], 'atlas')).toHaveLength(1)
    expect(filterCapabilities([entry], 'nothing')).toHaveLength(0)
    expect(filterCapabilities([entry], '  ')).toHaveLength(1)
  })
})
