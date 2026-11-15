import { describe, expect, it } from 'vitest'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { displayAgent, isDefaultAgent, ownAgents } from './agent-identity'

const nandi = {
  ...displayAgent(DEFAULT_AGENT, 'Nandi'),
  kind: 'default' as const,
}
const writer = displayAgent('writer', 'Writer')
const reviewer = {
  ...displayAgent('pr-reviewer', 'PR Reviewer'),
  kind: 'pack' as const,
}

describe('isDefaultAgent', () => {
  it('is true for Nandi only', () => {
    expect(isDefaultAgent(nandi)).toBe(true)
    expect(isDefaultAgent(writer)).toBe(false)
    expect(isDefaultAgent(reviewer)).toBe(false)
  })
})

describe('ownAgents', () => {
  it('keeps the person’s own and installed agents, never Nandi', () => {
    expect(ownAgents([nandi, writer, reviewer])).toEqual([writer, reviewer])
    expect(ownAgents([nandi])).toEqual([])
  })
})
