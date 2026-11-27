import { describe, expect, it } from 'vitest'
import {
  MAX_MENTIONS,
  mentionOptions,
  mentionTarget,
  mentionables,
  withMention,
} from './mention-model'

const agents = [
  { id: 'researcher', name: 'Researcher' },
  { id: 'research-lead', name: 'Research Lead' },
  { id: 'writer', name: 'Writer' },
]

describe('mentionOptions', () => {
  it('lists the agents a lone @word starts', () => {
    expect(mentionOptions('@', agents).map(a => a.id)).toEqual([
      'researcher',
      'research-lead',
      'writer',
    ])
    expect(mentionOptions('@res', agents).map(a => a.id)).toEqual([
      'researcher',
      'research-lead',
    ])
    expect(mentionOptions('@WRI', agents).map(a => a.id)).toEqual(['writer'])
  })

  it('stops once the message goes on, or without an @', () => {
    expect(mentionOptions('@Writer draft this', agents)).toEqual([])
    expect(mentionOptions('hello', agents)).toEqual([])
    const many = Array.from({ length: 10 }, (_, i) => ({
      id: `a${i}`,
      name: `A${i}`,
    }))
    expect(mentionOptions('@a', many)).toHaveLength(MAX_MENTIONS)
  })
})

describe('withMention', () => {
  it('is the name, then a space for the message', () => {
    expect(withMention({ id: 'writer', name: 'Writer' })).toBe('@Writer ')
    expect(withMention({ id: 'writer', name: ' ' })).toBe('@writer ')
  })
})

describe('mentionTarget', () => {
  it('names the agent and keeps the message', () => {
    expect(mentionTarget('@Writer draft the post', agents)).toEqual({
      id: 'writer',
      kind: 'agent',
      message: 'draft the post',
    })
    expect(mentionTarget('  @writer: hi', agents)).toEqual({
      id: 'writer',
      kind: 'agent',
      message: 'hi',
    })
  })

  it('takes the longest name, at a word boundary', () => {
    expect(mentionTarget('@Research Lead plan it', agents)?.id).toBe(
      'research-lead'
    )
    expect(mentionTarget('@Researcher, look', agents)?.id).toBe('researcher')
    expect(mentionTarget('@Writers unite', agents)).toBeNull()
    expect(mentionTarget('no mention', agents)).toBeNull()
  })
})

describe('people and groups', () => {
  const everyone = [
    ...agents,
    { id: 'inst-sam', name: 'Sam Lee', kind: 'person' as const, online: true },
    { id: 'g-fin', name: 'Finance', kind: 'group' as const },
  ]

  it('lists people and groups with the agents', () => {
    expect(mentionOptions('@', everyone).map(m => m.id)).toContain('inst-sam')
    expect(mentionOptions('@fin', everyone).map(m => m.id)).toEqual(['g-fin'])
  })

  it('says who a message is for: an agent, a person or a group', () => {
    expect(mentionTarget('@Sam Lee can you review Q4?', everyone)).toEqual({
      id: 'inst-sam',
      kind: 'person',
      message: 'can you review Q4?',
    })
    expect(
      mentionTarget('@Finance who owns the vendor list?', everyone)
    ).toEqual({
      id: 'g-fin',
      kind: 'group',
      message: 'who owns the vendor list?',
    })
  })
})

describe('mentionables', () => {
  it('adds the people on the relay but me, and my groups, after the agents', () => {
    const list = mentionables(
      [{ id: 'writer', name: 'Writer' }],
      [
        { instanceId: 'me', displayName: 'Ada', online: true },
        { instanceId: 'inst-sam', displayName: 'Sam', online: false },
        { instanceId: 'inst-x', displayName: ' ', online: true },
      ],
      [{ id: 'g1', name: 'Finance' }],
      'me'
    )
    expect(list.map(m => `${m.kind}:${m.id}`)).toEqual([
      'agent:writer',
      'person:inst-sam',
      'group:g1',
    ])
    expect(mentionables([], [], [], null)).toEqual([])
  })
})
