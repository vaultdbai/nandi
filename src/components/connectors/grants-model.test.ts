import { describe, expect, it } from 'vitest'
import { DEFAULT_AGENT, type GrantInfo } from '@/lib/bindings'
import {
  agentGrants,
  grantableConnectors,
  groupGrants,
  isMailConnector,
  mailboxChoices,
  mailboxCover,
  mailboxesToSave,
  type GrantNames,
} from './grants-model'

const NAMES: GrantNames = {
  agent: id => ({ [DEFAULT_AGENT]: 'Nandi', bot: 'Bot' })[id],
  connector: id => ({ tracker: 'Issue tracker' })[id],
}

const grant = (
  agent: string,
  connector: string,
  access: GrantInfo['access'] = 'read',
  scope: GrantInfo['scope'] = 'saved'
): GrantInfo => ({ agent, connector, access, scope, mailboxes: [] })

describe('groupGrants', () => {
  it('groups by agent with names, read before write', () => {
    const groups = groupGrants(
      [
        grant(DEFAULT_AGENT, 'tracker', 'write'),
        grant(DEFAULT_AGENT, 'outlook'),
        grant('bot', 'tracker', 'read', 'local'),
        grant(DEFAULT_AGENT, 'tracker'),
      ],
      NAMES
    )
    expect(groups.map(g => g.agentName)).toEqual(['Bot', 'Nandi'])
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
      grant(DEFAULT_AGENT, 'tracker', 'write'),
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

describe('mailboxes a mail grant covers', () => {
  it('knows the mail connectors', () => {
    expect(isMailConnector('gmail')).toBe(true)
    expect(isMailConnector('outlook')).toBe(true)
    expect(isMailConnector('google-calendar')).toBe(false)
  })

  it('covers every mailbox unless it names some', () => {
    expect(mailboxCover(grant(DEFAULT_AGENT, 'gmail'))).toEqual({
      kind: 'every',
    })
    expect(
      mailboxCover({ ...grant(DEFAULT_AGENT, 'gmail'), mailboxes: ['a@x.com'] })
    ).toEqual({ kind: 'only', mailboxes: ['a@x.com'] })
  })

  it('offers the connected mailboxes of the grant’s provider', () => {
    const connected = [
      { provider: 'gmail', email: 'Me@Home.com' },
      { provider: 'outlook', email: 'me@work.com' },
    ]
    expect(mailboxChoices(grant(DEFAULT_AGENT, 'gmail'), connected)).toEqual([
      'me@home.com',
    ])
  })

  it('saves every mailbox as none named, and refuses none ticked', () => {
    expect(mailboxesToSave(true, ['a@x.com'])).toEqual([])
    expect(mailboxesToSave(false, ['a@x.com'])).toEqual(['a@x.com'])
    expect(mailboxesToSave(false, [])).toBeNull()
  })
})
