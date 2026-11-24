import { describe, expect, it } from 'vitest'
import type { KeyView } from '@/lib/bindings'
import {
  emptyForm,
  formOf,
  formProblem,
  headerPreset,
  isBoundHost,
  keyWarning,
  parseHosts,
  rulesOf,
} from './lock-box-model'

const key = (patch: Partial<KeyView> = {}): KeyView => ({
  name: 'github',
  rules: {
    kind: 'token',
    hosts: ['api.github.com'],
    agents: ['writer'],
    lock: 'ask',
    unlockMinutes: 15,
    header: { name: 'authorization', prefix: 'Bearer ' },
  },
  stored: true,
  createdAt: 1,
  lastUsedAt: null,
  lastUsedBy: null,
  lastUsedHost: null,
  unlockedUntil: null,
  ...patch,
})

describe('lock box model', () => {
  it('asks each time by default and puts the key in a Bearer header', () => {
    const rules = rulesOf({ ...emptyForm(), name: 'github' })
    expect(rules.lock).toBe('ask')
    expect(rules.header).toEqual({ name: 'authorization', prefix: 'Bearer ' })
    expect(rules.hosts).toEqual([])
  })

  it('reads sites one per line or comma, lowercase, once each', () => {
    expect(
      parseHosts('API.github.com\n, api.github.com ,*.example.com\n')
    ).toEqual(['api.github.com', '*.example.com'])
  })

  it('takes a site name, never a URL, a port or a bare wildcard', () => {
    for (const ok of ['api.github.com', '*.example.com', 'bücher.de'])
      expect(isBoundHost(ok)).toBe(true)
    for (const bad of [
      'https://api.github.com',
      'api.github.com:443',
      'api.github.com/x',
      '*',
      '*.com',
      'com',
      'api.github.com.',
      'a.*.example.com',
    ])
      expect(isBoundHost(bad)).toBe(false)
  })

  it('says early what the core will refuse', () => {
    const form = { ...emptyForm(), name: 'github', hosts: 'api.github.com' }
    expect(formProblem(form, 'v')).toBeNull()
    expect(formProblem(form, null)).toBeNull()
    expect(formProblem({ ...form, name: 'Has Space' }, 'v')).toBe(
      'lockBox.problem.name'
    )
    expect(formProblem(form, ' ')).toBe('lockBox.problem.value')
    expect(formProblem(form, 'a\nb')).toBe('lockBox.problem.valueLines')
    expect(formProblem({ ...form, hosts: 'https://x.com' }, 'v')).toBe(
      'lockBox.problem.host'
    )
    expect(
      formProblem({ ...form, header: 'custom', customHeader: 'a b' }, 'v')
    ).toBe('lockBox.problem.header')
  })

  it('reads a saved key back into the form it was made with', () => {
    const form = formOf(key())
    expect(form.header).toBe('bearer')
    expect(rulesOf(form)).toEqual(key().rules)
    expect(headerPreset({ name: 'x-api-key', prefix: '' }).preset).toBe(
      'apiKey'
    )
    expect(headerPreset({ name: 'x-token', prefix: 'Token ' })).toEqual({
      preset: 'custom',
      name: 'x-token',
      prefix: 'Token ',
    })
    expect(headerPreset(null).preset).toBe('none')
  })

  it('warns about a key that can never be released', () => {
    expect(keyWarning(key())).toBeNull()
    expect(keyWarning(key({ stored: false }))).toBe('lockBox.warning.missing')
    expect(keyWarning(key({ rules: { ...key().rules, hosts: [] } }))).toBe(
      'lockBox.warning.noSites'
    )
    expect(keyWarning(key({ rules: { ...key().rules, agents: [] } }))).toBe(
      'lockBox.warning.noAgents'
    )
    expect(keyWarning(key({ rules: { ...key().rules, header: null } }))).toBe(
      'lockBox.warning.noHeader'
    )
  })
})
