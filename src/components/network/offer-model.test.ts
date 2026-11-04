import { describe, expect, it } from 'vitest'
import type { GatewayPolicy } from '@/lib/bindings'
import {
  applyOffer,
  buildOffer,
  capabilityColumn,
  MAX_RESPONSIBILITIES,
  MAX_RESPONSIBILITY_CHARS,
  offerChanges,
  parseResponsibilities,
  publicAbout,
} from './offer-model'

const policy = (change: Partial<GatewayPolicy> = {}): GatewayPolicy => ({
  maxConcurrent: 3,
  rateLimitPerHour: 30,
  requireApproval: [],
  expose: [],
  hide: [],
  ...change,
})

describe('capabilityColumn', () => {
  it('offers everything when nothing is exposed or hidden', () => {
    expect(capabilityColumn('summarize', policy())).toBe('offered')
  })

  it('hides what is hidden, and what an explicit expose leaves out', () => {
    expect(capabilityColumn('notes', policy({ hide: ['notes'] }))).toBe(
      'hidden'
    )
    expect(capabilityColumn('notes', policy({ expose: ['summarize'] }))).toBe(
      'hidden'
    )
  })

  it('puts approval after hidden', () => {
    expect(
      capabilityColumn('email', policy({ requireApproval: ['email'] }))
    ).toBe('approval')
    expect(
      capabilityColumn(
        'email',
        policy({ requireApproval: ['email'], hide: ['email'] })
      )
    ).toBe('hidden')
  })
})

describe('buildOffer', () => {
  it('names every agent serving a capability, sorted', () => {
    const offer = buildOffer(
      [
        { agent: 'Writer', capabilities: ['summarize', 'email'] },
        { agent: 'Reader', capabilities: ['summarize'] },
      ],
      policy({ requireApproval: ['email'] })
    )
    expect(offer).toEqual([
      { name: 'email', agents: ['Writer'], column: 'approval' },
      { name: 'summarize', agents: ['Reader', 'Writer'], column: 'offered' },
    ])
  })
})

describe('applyOffer', () => {
  it('hides through hide while expose is empty', () => {
    const next = applyOffer(policy({ requireApproval: ['email'] }), {
      summarize: 'hidden',
      email: 'offered',
    })
    expect(next.expose).toEqual([])
    expect(next.hide).toEqual(['summarize'])
    expect(next.requireApproval).toEqual([])
  })

  it('keeps an explicit expose explicit, and policy names no agent serves', () => {
    const next = applyOffer(
      policy({ expose: ['remote-only', 'summarize'], hide: ['old'] }),
      { summarize: 'approval', notes: 'offered', email: 'hidden' }
    )
    expect(next.expose).toEqual(['remote-only', 'notes', 'summarize'])
    expect(next.requireApproval).toEqual(['summarize'])
    expect(next.hide).toEqual(['old', 'email'])
  })
})

describe('offerChanges', () => {
  it('lists only what moved', () => {
    const before = buildOffer(
      [{ agent: 'Writer', capabilities: ['a', 'b'] }],
      policy()
    )
    expect(offerChanges(before, { a: 'offered', b: 'hidden' })).toEqual([
      { name: 'b', column: 'hidden' },
    ])
  })
})

describe('parseResponsibilities', () => {
  it('keeps one per line, trimmed, without blanks or repeats', () => {
    expect(
      parseResponsibilities('Risk\n  SOX audit  \r\n\nrisk\nbilling system\n')
    ).toEqual(['Risk', 'SOX audit', 'billing system'])
    expect(parseResponsibilities('')).toEqual([])
  })

  it('caps each line and the list', () => {
    const long = 'é'.repeat(MAX_RESPONSIBILITY_CHARS + 5)
    const many = Array.from({ length: 80 }, (_, i) => `area ${i}`)
    const kept = parseResponsibilities([long, ...many].join('\n'))
    expect(kept).toHaveLength(MAX_RESPONSIBILITIES)
    expect(Array.from(kept[0] ?? '')).toHaveLength(MAX_RESPONSIBILITY_CHARS)
    expect(kept[1]).toBe('area 0')
  })
})

describe('publicAbout', () => {
  it('leaves out the private comments, keeps the rest', () => {
    const body = [
      '<!-- Your public profile.',
      'Comments like this one stay private. -->',
      '',
      "I'm Lena.",
      '',
      '',
      '',
      '## What I can help with',
    ].join('\n')
    expect(publicAbout('Data platform', body)).toBe(
      "Data platform\n\nI'm Lena.\n\n## What I can help with"
    )
  })

  it('hides an unclosed comment and handles an empty profile', () => {
    expect(publicAbout('', 'Hi <!-- draft')).toBe('Hi')
    expect(publicAbout('', '<!-- only a comment -->')).toBe('')
  })
})
