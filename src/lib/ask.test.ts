import { describe, expect, it } from 'vitest'
import type { View } from '@/store/navigation-store'
import { askContextForView, sourceView } from './ask'

describe('askContextForView', () => {
  const cases: [View, ReturnType<typeof askContextForView>][] = [
    [
      { kind: 'home' },
      { currentView: 'home', focusedArtifact: null, recentRuns: [] },
    ],
    [
      { kind: 'agent', agentId: 'writer' },
      {
        currentView: 'agent:writer',
        focusedArtifact: { kind: 'agent', id: 'writer' },
        recentRuns: [],
      },
    ],
    [
      { kind: 'capability-detail', id: 'code-review.python' },
      {
        currentView: 'capability:code-review.python',
        focusedArtifact: { kind: 'capability', id: 'code-review.python' },
        recentRuns: [],
      },
    ],
    [
      { kind: 'workflow', workflowId: 'onboarding' },
      {
        currentView: 'workflow:onboarding',
        focusedArtifact: { kind: 'workflow', id: 'onboarding' },
        recentRuns: [],
      },
    ],
    [
      { kind: 'member-profile', instanceId: 'alice' },
      { currentView: 'member:alice', focusedArtifact: null, recentRuns: [] },
    ],
    [
      { kind: 'run-detail', runId: 'r1' },
      { currentView: 'run-detail', focusedArtifact: null, recentRuns: ['r1'] },
    ],
    [
      { kind: 'network', tab: 'sharing' },
      {
        currentView: 'gateway',
        focusedArtifact: { kind: 'gateway' },
        recentRuns: [],
      },
    ],
    [
      { kind: 'network', tab: 'people' },
      { currentView: 'network', focusedArtifact: null, recentRuns: [] },
    ],
    [
      { kind: 'starter-packs' },
      { currentView: 'starter-packs', focusedArtifact: null, recentRuns: [] },
    ],
  ]

  it.each(cases)('%o', (view, context) => {
    expect(askContextForView(view)).toEqual(context)
  })

  it('opens sources in their views', () => {
    expect(sourceView({ kind: 'run', id: 'r1', label: 'R' })).toEqual({
      kind: 'run-detail',
      runId: 'r1',
    })
    expect(sourceView({ kind: 'gateway', id: '', label: 'G' })).toEqual({
      kind: 'network',
      tab: 'sharing',
      sharing: 'offer',
    })
    expect(sourceView({ kind: 'policy', id: 'p', label: 'P' })).toBeNull()
  })
})
