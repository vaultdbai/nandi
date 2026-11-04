import { describe, expect, it } from 'vitest'
import { networkSnapshot } from './task-fixtures'
import { agentLabel, memberName } from './task-utils'

describe('agentLabel', () => {
  it('names the member, never the device ID', () => {
    expect(agentLabel('papers', 'Peer Person')).toBe('papers · Peer Person')
    expect(agentLabel('papers', null)).toBe('papers')
  })
})

describe('memberName', () => {
  it('reads the display name from the directory', () => {
    expect(memberName(networkSnapshot(true), 'peer')).toBe('Peer Person')
  })

  it('is null for unknown instances or without a directory', () => {
    expect(memberName(networkSnapshot(true), 'stranger')).toBeNull()
    expect(memberName(undefined, 'peer')).toBeNull()
    expect(memberName(networkSnapshot(true), '')).toBeNull()
  })
})
