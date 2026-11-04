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
  }),
  { sources: ['team'], teamIds: ['acme'] }
)
const atlas = sharedAgent(
  agentCard('bob', 'atlas', {
    name: 'Atlas',
    capabilities: [{ name: 'geocode' }],
  }),
  { sources: ['relay'] }
)
const both = sharedAgent(agentCard('carol', 'scribe', { name: 'Scribe' }), {
  sources: ['team', 'relay'],
  teamIds: ['beta'],
})
const all = [papers, atlas, both]
const names = (list: typeof all) => list.map(a => a.card.name)

describe('filterSharedAgents', () => {
  it('keeps everything with no filter', () => {
    expect(
      names(filterSharedAgents(all, { source: 'all', search: '' }))
    ).toEqual(['Papers', 'Atlas', 'Scribe'])
  })

  it('filters by source, keeping agents found in both', () => {
    expect(
      names(filterSharedAgents(all, { source: 'team', search: '' }))
    ).toEqual(['Papers', 'Scribe'])
    expect(
      names(filterSharedAgents(all, { source: 'relay', search: '' }))
    ).toEqual(['Atlas', 'Scribe'])
  })

  it('searches name, description, owner, capability, and team, ignoring case', () => {
    const search = (text: string) =>
      names(filterSharedAgents(all, { source: 'all', search: text }))
    expect(search('  ATLAS ')).toEqual(['Atlas'])
    expect(search('research')).toEqual(['Papers'])
    expect(search('bob-owner')).toEqual(['Atlas'])
    expect(search('geocode')).toEqual(['Atlas'])
    expect(search('beta')).toEqual(['Scribe'])
    expect(search('nothing-like-it')).toEqual([])
  })

  it('combines source and search', () => {
    expect(
      names(filterSharedAgents(all, { source: 'relay', search: 'find-papers' }))
    ).toEqual(['Scribe'])
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
      providerOption(papers.card, { teamIds: ['acme'] }),
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

  it('filters by capability, provider, owner, or team', () => {
    expect(filterCapabilities([entry], 'PAPERS')).toHaveLength(1)
    expect(filterCapabilities([entry], 'bob-owner')).toHaveLength(1)
    expect(filterCapabilities([entry], 'acme')).toHaveLength(1)
    expect(filterCapabilities([entry], 'nothing')).toHaveLength(0)
    expect(filterCapabilities([entry], '  ')).toHaveLength(1)
  })
})
