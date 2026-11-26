import { describe, expect, it } from 'vitest'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { approvalSummary } from '@/components/inbox/approval-model'
import {
  accessChoices,
  accessSentence,
  agentLabel,
  bareSite,
  answerMailboxes,
  canChooseMailboxes,
  decidedKey,
  parseAccessRequest,
  rawSiteHost,
} from './access-model'

const ARGS = {
  agent: DEFAULT_AGENT,
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
      agent: DEFAULT_AGENT,
      family: 'email',
      connectors: [{ id: 'outlook', name: 'Outlook' }],
      access: 'read',
      tool: 'email_search',
      details: ['Mail in your inbox'],
      askEveryTime: false,
      choice: null,
      decidedBy: null,
      site: null,
      mailboxes: [],
      grantedMailboxes: null,
    })
  })

  it('reads the mailboxes a request reaches and what its grant covers', () => {
    const parsed = parseAccessRequest({
      ...ARGS,
      mailboxes: ['me@work.com', '', 'me@home.com'],
      grantedMailboxes: ['me@work.com'],
    })
    expect(parsed?.mailboxes).toEqual(['me@work.com', 'me@home.com'])
    expect(parsed?.grantedMailboxes).toEqual(['me@work.com'])
  })

  it('lets the person choose mailboxes only when there are several', () => {
    const one = parseAccessRequest({ ...ARGS, mailboxes: ['me@work.com'] })
    expect(one && canChooseMailboxes(one)).toBe(false)
    expect(one && answerMailboxes(one, [])).toBeNull()
    const two = parseAccessRequest({
      ...ARGS,
      mailboxes: ['me@work.com', 'me@home.com'],
    })
    expect(two && canChooseMailboxes(two)).toBe(true)
    expect(two && answerMailboxes(two, [])).toEqual([
      'me@work.com',
      'me@home.com',
    ])
    expect(two && answerMailboxes(two, ['me@home.com'])).toEqual([
      'me@work.com',
    ])
    const calendar = parseAccessRequest({
      ...ARGS,
      family: 'calendar',
      mailboxes: ['a@x.com', 'b@y.com'],
    })
    expect(calendar && canChooseMailboxes(calendar)).toBe(false)
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
    expect(accessSentence(request, 'Nandi')).toEqual({
      key: 'emailRead',
      values: { agent: 'Nandi', names: 'Outlook' },
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
      values: { agent: 'Nandi', names: 'Outlook' },
    })
  })
})

describe('agentLabel', () => {
  it('turns an id into a name', () => {
    expect(agentLabel('pr-reviewer')).toBe('PR Reviewer')
  })
})

const SITE_ARGS = {
  agent: 'browser',
  family: 'site',
  connectors: [],
  access: 'read',
  tool: 'pilot_browser__browser_navigate',
  details: [],
  askEveryTime: false,
  site: {
    host: 'xn--bcher-kva.example',
    display: 'bücher.example',
    pilot: 'browser',
    reason: ' Find the hours ',
    canSave: true,
  },
}

describe('a site request', () => {
  it('reads the site, and refuses a site request without one', () => {
    expect(parseAccessRequest(SITE_ARGS)?.site).toEqual({
      host: 'xn--bcher-kva.example',
      display: 'bücher.example',
      pilot: 'browser',
      reason: 'Find the hours',
      canSave: true,
      subdomains: false,
      also: [],
    })
    expect(parseAccessRequest({ ...SITE_ARGS, site: null })).toBeNull()
    expect(parseAccessRequest({ ...SITE_ARGS, site: { host: '' } })).toBeNull()
  })

  it('names the site as people read it, and the raw host when it differs', () => {
    const request = parseAccessRequest(SITE_ARGS)
    if (!request?.site) throw new Error('parse')
    expect(accessSentence(request, 'Browser')).toEqual({
      key: 'site',
      values: { agent: 'Browser', names: 'bücher.example' },
    })
    expect(rawSiteHost(request.site)).toBe('xn--bcher-kva.example')
    expect(
      rawSiteHost({ ...request.site, display: request.site.host })
    ).toBeNull()
  })

  it('offers once or always, and always only when the agent can change', () => {
    const request = parseAccessRequest(SITE_ARGS)
    if (!request?.site) throw new Error('parse')
    expect(accessChoices(request)).toEqual(['allowOnce', 'allowSaved', 'deny'])
    expect(
      accessChoices({ ...request, site: { ...request.site, canSave: false } })
    ).toEqual(['allowOnce', 'deny'])
  })

  it('is summed up in the Inbox by the site', () => {
    expect(approvalSummary('connector_access', SITE_ARGS)).toEqual({
      key: 'siteAccess',
      values: { agent: 'Browser', site: 'bücher.example' },
    })
  })

  it('names every host a page needed in one card', () => {
    const grouped = {
      ...SITE_ARGS,
      site: {
        ...SITE_ARGS.site,
        host: 'www.gstatic.com',
        display: 'www.gstatic.com',
        also: ['encrypted-tbn0.gstatic.com', '', 7],
      },
    }
    const request = parseAccessRequest(grouped)
    if (!request?.site) throw new Error('parse')
    expect(request.site.also).toEqual(['encrypted-tbn0.gstatic.com'])
    expect(accessSentence(request, 'Nandi').values.names).toBe(
      'www.gstatic.com, encrypted-tbn0.gstatic.com'
    )
    expect(approvalSummary('connector_access', grouped).values).toEqual({
      agent: 'Browser',
      site: 'www.gstatic.com, encrypted-tbn0.gstatic.com',
    })
  })
})

describe('bareSite', () => {
  it('reads www.example.com as example.com', () => {
    expect(bareSite('www.youtube.com')).toBe('youtube.com')
    expect(bareSite('music.youtube.com')).toBe('music.youtube.com')
    expect(bareSite('youtube.com')).toBe('youtube.com')
    expect(bareSite('www.localhost')).toBe('www.localhost')
  })
})
