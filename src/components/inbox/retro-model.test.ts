import { describe, expect, it } from 'vitest'
import type { RetroItem } from '@/lib/bindings'
import {
  isRetroWaiting,
  ratePercent,
  retroCardState,
  retroDiffLines,
  retroTargetLabel,
} from './retro-model'

const retroItem = (overrides: Partial<RetroItem> = {}): RetroItem => ({
  id: 'p1',
  agentId: 'writer',
  agentName: 'Writer',
  target: 'agent',
  path: '.nandi/agents/writer/agent.md',
  reason: 'You shortened 4 of 5 drafts',
  runs: ['r1', 'r2'],
  diff: '',
  status: 'pending',
  createdAt: 1000,
  decidedAt: null,
  expiresAt: 2000,
  commitId: null,
  verdict: null,
  rates: null,
  needsNote: false,
  ...overrides,
})

describe('retro-model', () => {
  it('tells a proposal from an applied change and a note', () => {
    expect(retroCardState(retroItem())).toBe('proposal')
    expect(isRetroWaiting(retroItem())).toBe(true)
    const applied = retroItem({ status: 'applied', decidedAt: 1500 })
    expect(retroCardState(applied)).toBe('applied')
    expect(isRetroWaiting(applied)).toBe(false)
    expect(
      retroCardState(retroItem({ status: 'applied', needsNote: true }))
    ).toBe('didntHelp')
  })

  it('names what changes', () => {
    expect(retroTargetLabel('agent')).toEqual({
      key: 'retro.target.agent',
      values: {},
    })
    expect(retroTargetLabel('playbook:linkedin-posts')).toEqual({
      key: 'retro.target.playbook',
      values: { name: 'linkedin-posts' },
    })
    expect(retroTargetLabel('trigger:morning-digest').key).toBe(
      'retro.target.trigger'
    )
    expect(retroTargetLabel('workflow:weekly').values.name).toBe('weekly')
  })

  it('reads a unified diff without its headers', () => {
    const diff = [
      'diff --git a/agent.md b/agent.md',
      '--- a/agent.md',
      '+++ b/agent.md',
      '@@ -1,2 +1,3 @@',
      ' Draft posts.',
      '-Be thorough.',
      '+Keep them under 80 words.',
      '\\ No newline at end of file',
      '',
    ].join('\n')
    expect(retroDiffLines(diff)).toEqual([
      { kind: 'same', text: 'Draft posts.' },
      { kind: 'removed', text: 'Be thorough.' },
      { kind: 'added', text: 'Keep them under 80 words.' },
    ])
  })

  it('shows rates as whole percentages', () => {
    expect(ratePercent({ decisions: 4, corrections: 2 })).toBe(50)
    expect(ratePercent({ decisions: 3, corrections: 1 })).toBe(33)
    expect(ratePercent({ decisions: 0, corrections: 0 })).toBe(0)
  })
})
