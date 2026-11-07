import { describe, expect, it } from 'vitest'
import type { GrantInfo } from '@/lib/bindings'
import {
  agentGrants,
  grantableConnectors,
  groupGrants,
  type GrantNames,
} from './grants-model'

const NAMES: GrantNames = {
  agent: id => ({ 'chief-of-staff': 'Chief of Staff', bot: 'Bot' })[id],
  connector: id => ({ tracker: 'Issue tracker' })[id],
}

const grant = (
  agent: string,
  connector: string,
  access: GrantInfo['access'] = 'read',
  scope: GrantInfo['scope'] = 'saved'
): GrantInfo => ({ agent, connector, access, scope })

describe('groupGrants', () => {
  it('groups by agent with names, read before write', () => {
    const groups = groupGrants(
      [
        grant('chief-of-staff', 'tracker', 'write'),
        grant('chief-of-staff', 'outlook'),
        grant('bot', 'tracker', 'read', 'local'),
        grant('chief-of-staff', 'tracker'),
      ],
      NAMES
    )
    expect(groups.map(g => g.agentName)).toEqual(['Bot', 'Chief of Staff'])
    expect(
      groups[1]?.rows.map(r => `${r.connectorName}:${r.grant.access}`)
    ).toEqual(['Issue tracker:read', 'Issue tracker:write', 'Outlook:read'])
  })

  it('falls back to ids for unknown agents and connectors', () => {
    const [group] = groupGrants([grant('ghost', 'mystery')], NAMES)
    expect(group?.agentName).toBe('ghost')
    expect(group?.rows[0]?.connectorName).toBe('mystery')
  })

  it('is empty without grants', () => {
    expect(groupGrants([], NAMES)).toEqual([])
  })
})

describe('agentGrants', () => {
  it("splits one agent's grants on a connector by access", () => {
    const grants = [
      grant('bot', 'tracker', 'read', 'local'),
      grant('bot', 'tracker', 'read', 'saved'),
      grant('bot', 'other'),
      grant('chief-of-staff', 'tracker', 'write'),
    ]
    const mine = agentGrants(grants, 'bot', 'tracker')
    expect(mine.read.map(g => g.scope)).toEqual(['saved', 'local'])
    expect(mine.write).toEqual([])
  })
})

describe('grantableConnectors', () => {
  it('lists package connectors and connected accounts once', () => {
    expect(
      grantableConnectors(
        [
          { id: 'tracker', name: 'Issue tracker', package: { version: '1' } },
          { id: 'github', name: 'GitHub', package: null },
        ],
        ['outlook', 'onedrive']
      )
    ).toEqual([
      { id: 'tracker', name: 'Issue tracker' },
      { id: 'outlook', name: 'Outlook' },
      { id: 'onedrive', name: 'OneDrive' },
    ])
  })
})
