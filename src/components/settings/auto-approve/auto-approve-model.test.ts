import { describe, expect, it } from 'vitest'
import type { AutoApproveRule } from '@/lib/bindings'
import {
  MAX_RULES,
  actionsFor,
  addRule,
  checkNewRule,
  checkRule,
  domainShortcut,
  draftToRule,
  emptyDraft,
  newRuleId,
  normalizeDomain,
  removeRule,
  ruleLabel,
  senderAddress,
  senderDomain,
  toggleAction,
  toggleRuleAction,
} from './auto-approve-model'

const domainRule = (
  domain: string,
  actions: AutoApproveRule['actions'] = ['start']
): AutoApproveRule => ({
  id: `id-${domain}`,
  match: { kind: 'emailDomain', domain },
  actions,
})

describe('auto-approve-model', () => {
  it('offers mail the start and update actions, a teammate the run action', () => {
    expect(actionsFor('emailDomain')).toEqual(['start', 'update'])
    expect(actionsFor('emailAddress')).toEqual(['start', 'update'])
    expect(actionsFor('teammate')).toEqual(['run'])
  })

  it('names each kind of rule in plain words', () => {
    expect(ruleLabel(domainRule('chase.com').match)).toEqual({
      key: 'autoApprove.label.emailDomain',
      values: { domain: 'chase.com' },
    })
    expect(
      ruleLabel({ kind: 'emailAddress', address: 'billing@acme-energy.com' })
    ).toEqual({
      key: 'autoApprove.label.emailAddress',
      values: { address: 'billing@acme-energy.com' },
    })
    expect(
      ruleLabel({ kind: 'teammate', instanceId: 'i1', name: 'Lena Park' })
    ).toEqual({
      key: 'autoApprove.label.teammate',
      values: { name: 'Lena Park' },
    })
  })

  it('makes a new ID each time', () => {
    expect(newRuleId()).not.toEqual(newRuleId())
  })

  it('reads the address and domain out of a sender', () => {
    expect(senderAddress('Chase <Alerts@Chase.com>')).toBe('alerts@chase.com')
    expect(senderAddress('bob@acme.io')).toBe('bob@acme.io')
    expect(senderAddress('Nobody')).toBeNull()
    expect(senderAddress(null)).toBeNull()
    expect(senderDomain('Chase <alerts@alerts.chase.com>')).toBe(
      'alerts.chase.com'
    )
    expect(senderDomain('Nobody')).toBeNull()
  })

  it('makes the always-allow rule from a queued item', () => {
    const rule = domainShortcut('chase.com', 'update')
    expect(rule.match).toEqual({ kind: 'emailDomain', domain: 'chase.com' })
    expect(rule.actions).toEqual(['update'])
    expect(checkRule(rule)).toBeNull()
  })

  it('cleans up a typed domain', () => {
    expect(normalizeDomain('  @Chase.COM ')).toBe('chase.com')
  })

  describe('checkRule', () => {
    it('wants an action', () => {
      expect(checkRule(domainRule('chase.com', []))).toBe(
        'autoApprove.error.noActions'
      )
    })
    it('refuses an action the kind cannot take', () => {
      expect(checkRule(domainRule('chase.com', ['run']))).toBe(
        'autoApprove.error.wrongAction'
      )
    })
    it('checks the domain', () => {
      expect(checkRule(domainRule('chase'))).toBe('autoApprove.error.domain')
      expect(checkRule(domainRule('https://chase.com'))).toBe(
        'autoApprove.error.domain'
      )
      expect(checkRule(domainRule('alerts.chase.com'))).toBeNull()
    })
    it('checks the address', () => {
      const rule = (address: string): AutoApproveRule => ({
        id: 'a',
        match: { kind: 'emailAddress', address },
        actions: ['start'],
      })
      expect(checkRule(rule('billing'))).toBe('autoApprove.error.address')
      expect(checkRule(rule('billing@acme-energy.com'))).toBeNull()
    })
    it('wants a teammate picked', () => {
      const rule: AutoApproveRule = {
        id: 'a',
        match: { kind: 'teammate', instanceId: '', name: '' },
        actions: ['run'],
      }
      expect(checkRule(rule)).toBe('autoApprove.error.teammate')
    })
  })

  describe('draftToRule', () => {
    it('stays empty until something is typed or picked', () => {
      expect(draftToRule(emptyDraft('emailDomain'))).toBeNull()
      expect(draftToRule(emptyDraft('teammate'))).toBeNull()
    })
    it('builds a domain rule from typed text', () => {
      const rule = draftToRule({
        ...emptyDraft('emailDomain'),
        value: ' Chase.com ',
      })
      expect(rule?.match).toEqual({ kind: 'emailDomain', domain: 'chase.com' })
      expect(rule?.actions).toEqual(['start', 'update'])
    })
    it('builds a teammate rule from the pick', () => {
      const rule = draftToRule({
        ...emptyDraft('teammate'),
        teammate: { instanceId: 'i1', name: 'Lena Park' },
      })
      expect(rule?.match).toEqual({
        kind: 'teammate',
        instanceId: 'i1',
        name: 'Lena Park',
      })
      expect(rule?.actions).toEqual(['run'])
    })
  })

  it('switches actions on and off in a stable order', () => {
    expect(toggleAction(['update'], 'start')).toEqual(['start', 'update'])
    expect(toggleAction(['start', 'update'], 'start')).toEqual(['update'])
  })

  describe('the list', () => {
    it('adds a rule', () => {
      expect(addRule([], domainRule('chase.com'))).toHaveLength(1)
    })
    it('merges a rule for a sender already in the list', () => {
      const rules = addRule(
        [domainRule('chase.com', ['update'])],
        domainRule('chase.com', ['start'])
      )
      expect(rules).toHaveLength(1)
      expect(rules[0]?.actions).toEqual(['start', 'update'])
    })
    it('removes a rule by ID', () => {
      const rules = removeRule(
        [domainRule('a.com'), domainRule('b.com')],
        'id-a.com'
      )
      expect(rules.map(r => r.id)).toEqual(['id-b.com'])
    })
    it('keeps at least one action on a rule', () => {
      const rules = [domainRule('chase.com', ['start'])]
      expect(toggleRuleAction(rules, 'id-chase.com', 'start')).toEqual(rules)
      expect(
        toggleRuleAction(rules, 'id-chase.com', 'update')[0]?.actions
      ).toEqual(['start', 'update'])
    })
    it('refuses the 101st rule, but still merges into a full list', () => {
      const full = Array.from({ length: MAX_RULES }, (_, i) =>
        domainRule(`site${i}.com`)
      )
      expect(checkNewRule(full, domainRule('new.com'))).toBe(
        'autoApprove.error.full'
      )
      expect(checkNewRule(full, domainRule('site1.com'))).toBeNull()
    })
  })
})
