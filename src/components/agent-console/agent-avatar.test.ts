import { describe, it, expect } from 'vitest'
import { getAgentAvatarClassName, getAgentInitials } from './agent-avatar'

describe('getAgentInitials', () => {
  it('uses the first letter of the first two words', () => {
    expect(getAgentInitials('Research Agent')).toBe('RA')
    expect(getAgentInitials('code-reviewer bot')).toBe('CR')
  })

  it('uses the first two letters of a single word', () => {
    expect(getAgentInitials('writer')).toBe('WR')
  })

  it('falls back for empty names', () => {
    expect(getAgentInitials('')).toBe('?')
  })
})

describe('getAgentAvatarClassName', () => {
  it('is stable for the same ID', () => {
    expect(getAgentAvatarClassName('research')).toBe(
      getAgentAvatarClassName('research')
    )
  })

  it('returns a palette class', () => {
    expect(getAgentAvatarClassName('anything')).toMatch(/^bg-/)
  })
})
