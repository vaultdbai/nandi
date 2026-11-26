import { describe, expect, it } from 'vitest'
import {
  MAX_MENTIONS,
  mentionOptions,
  mentionTarget,
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
      agentId: 'writer',
      message: 'draft the post',
    })
    expect(mentionTarget('  @writer: hi', agents)).toEqual({
      agentId: 'writer',
      message: 'hi',
    })
  })

  it('takes the longest name, at a word boundary', () => {
    expect(mentionTarget('@Research Lead plan it', agents)?.agentId).toBe(
      'research-lead'
    )
    expect(mentionTarget('@Researcher, look', agents)?.agentId).toBe(
      'researcher'
    )
    expect(mentionTarget('@Writers unite', agents)).toBeNull()
    expect(mentionTarget('no mention', agents)).toBeNull()
  })
})
