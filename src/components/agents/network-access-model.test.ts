import { describe, expect, it } from 'vitest'
import type { EgressLogEntry } from '@/lib/tauri-bindings'
import { accessMode, blockedSite, withSite } from './network-access-model'

const entry = (patch: Partial<EgressLogEntry>): EgressLogEntry => ({
  ts: 0,
  agentId: 'writer',
  kind: 'fetch',
  target: 'GET https://api.example.com/v1',
  allowed: false,
  status: null,
  durationMs: null,
  error: null,
  ...patch,
})

describe('accessMode', () => {
  it('reads none, some, or any site', () => {
    expect(accessMode([])).toBe('none')
    expect(accessMode([' '])).toBe('none')
    expect(accessMode(['api.example.com'])).toBe('some')
    expect(accessMode(['api.example.com', '*'])).toBe('any')
  })
})

describe('withSite', () => {
  it('adds a new site, normalized', () => {
    expect(withSite(['a.com'], ' API.Example.com. ')).toEqual([
      'a.com',
      'api.example.com',
    ])
  })

  it('leaves the list alone when the site is there or every site is', () => {
    const sites = ['Api.Example.com']
    expect(withSite(sites, 'api.example.com')).toBe(sites)
    const any = ['*']
    expect(withSite(any, 'b.com')).toBe(any)
    expect(withSite(sites, ' ')).toBe(sites)
  })
})

describe('blockedSite', () => {
  it('takes the host of a blocked fetch, with a non-default port', () => {
    expect(blockedSite(entry({}))).toBe('api.example.com')
    expect(
      blockedSite(entry({ target: 'POST http://localhost:8080/hook' }))
    ).toBe('localhost:8080')
  })

  it('takes the host of a blocked tunnel, dropping port 443', () => {
    expect(blockedSite(entry({ kind: 'tunnel', target: 'Pypi.org:443' }))).toBe(
      'pypi.org'
    )
    expect(
      blockedSite(entry({ kind: 'tunnel', target: 'git.example.com:22' }))
    ).toBe('git.example.com:22')
  })

  it('has nothing to allow for allowed requests, AWS, or bad targets', () => {
    expect(blockedSite(entry({ allowed: true }))).toBeNull()
    expect(
      blockedSite(entry({ kind: 'aws', target: 's3:GetObject arn:aws:s3:::b' }))
    ).toBeNull()
    expect(blockedSite(entry({ target: 'GET not a url' }))).toBeNull()
  })
})
