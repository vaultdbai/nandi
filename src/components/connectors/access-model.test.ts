import { describe, expect, it } from 'vitest'
import { approvalSummary } from '@/components/inbox/approval-model'
import {
  accessChoices,
  accessSentence,
  agentLabel,
  decidedKey,
  parseAccessRequest,
} from './access-model'

const ARGS = {
  agent: 'chief-of-staff',
  family: 'email',
  connectors: [{ id: 'outlook', name: 'Outlook' }],
  access: 'read',
  tool: 'email_search',
  details: ['Mail in your inbox', ''],
  askEveryTime: false,
}

describe('parseAccessRequest', () => {
  it('reads the args of a connector_access approval', () => {
    expect(parseAccessRequest(ARGS)).toEqual({
      agent: 'chief-of-staff',
      family: 'email',
      connectors: [{ id: 'outlook', name: 'Outlook' }],
      access: 'read',
      tool: 'email_search',
      details: ['Mail in your inbox'],
      askEveryTime: false,
      choice: null,
      decidedBy: null,
    })
  })

  it('refuses args that are not an access request', () => {
    expect(parseAccessRequest(null)).toBeNull()
    expect(parseAccessRequest({ family: 'phone' })).toBeNull()
  })

  it('names a connector by its id when it has no name', () => {
    const parsed = parseAccessRequest({
      ...ARGS,
      connectors: [{ id: 'tracker' }, { name: 'no id' }],
    })
    expect(parsed?.connectors).toEqual([{ id: 'tracker', name: 'tracker' }])
  })

  it('keeps the choice and who decided', () => {
    const parsed = parseAccessRequest({
      ...ARGS,
      choice: 'allowSaved',
      decidedBy: 'Dev',
    })
    expect(parsed?.choice).toBe('allowSaved')
    expect(parsed?.decidedBy).toBe('Dev')
  })
})

describe('accessSentence', () => {
  it('says the family and access', () => {
    const request = parseAccessRequest(ARGS)
    if (!request) throw new Error('parse')
    expect(accessSentence(request, 'Chief of Staff')).toEqual({
      key: 'emailRead',
      values: { agent: 'Chief of Staff', names: 'Outlook' },
    })
    expect(
      accessSentence(
        { ...request, family: 'connector', access: 'write' },
        'Bot'
      ).key
    ).toBe('connectorWrite')
  })

  it('lists every account asked for', () => {
    const request = parseAccessRequest({
      ...ARGS,
      family: 'calendar',
      connectors: [
        { id: 'outlook-calendar', name: 'Outlook calendar' },
        { id: 'google-calendar', name: 'Google Calendar' },
      ],
    })
    if (!request) throw new Error('parse')
    expect(accessSentence(request, 'A').values.names).toBe(
      'Outlook calendar, Google Calendar'
    )
  })
})

describe('accessChoices', () => {
  it('offers every answer, or only once when a policy asks every time', () => {
    const request = parseAccessRequest(ARGS)
    if (!request) throw new Error('parse')
    expect(accessChoices(request)).toEqual([
      'allowOnce',
      'allowSaved',
      'allowLocal',
      'deny',
    ])
    expect(accessChoices({ ...request, askEveryTime: true })).toEqual([
      'allowOnce',
      'deny',
    ])
  })
})

describe('decidedKey', () => {
  it('prefers the recorded choice, else the status', () => {
    const request = parseAccessRequest(ARGS)
    if (!request) throw new Error('parse')
    expect(decidedKey({ ...request, choice: 'allowLocal' }, 'approved')).toBe(
      'allowLocal'
    )
    expect(decidedKey(request, 'approved')).toBe('allowed')
    expect(decidedKey(request, 'rejected')).toBe('deny')
    expect(decidedKey(request, 'expired')).toBe('expired')
  })
})

describe('approvalSummary for connector_access', () => {
  it('says who asks for what', () => {
    expect(approvalSummary('connector_access', ARGS)).toEqual({
      key: 'connectorAccess',
      values: { agent: 'Chief OF Staff', names: 'Outlook' },
    })
  })
})

describe('agentLabel', () => {
  it('turns an id into a name', () => {
    expect(agentLabel('pr-reviewer')).toBe('PR Reviewer')
  })
})
