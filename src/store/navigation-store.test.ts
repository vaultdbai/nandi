import { describe, it, expect, beforeEach } from 'vitest'
import { DEFAULT_AGENT } from '@/lib/bindings'
import {
  MAX_HISTORY,
  NAV_SECTIONS,
  SECTION_VIEWS,
  isActiveView,
  navSection,
  parseView,
  useNavigationStore,
  type NavSection,
  type View,
} from './navigation-store'

describe('NavigationStore', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('starts on Nandi with no history', () => {
    const state = useNavigationStore.getState()
    expect(state.current).toEqual({ kind: 'nandi' })
    expect(state.history).toEqual([])
  })

  it('navigates and goes back through the history stack', () => {
    const { navigate, goBack } = useNavigationStore.getState()
    navigate({ kind: 'agent', agentId: 'a1' })
    navigate({ kind: 'automations', tab: 'workflows' })

    goBack()
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'agent',
      agentId: 'a1',
    })
    goBack()
    expect(useNavigationStore.getState().current).toEqual({ kind: 'nandi' })
    // Nothing left: stays put
    goBack()
    expect(useNavigationStore.getState().current).toEqual({ kind: 'nandi' })
  })

  it('ignores navigation to the current view', () => {
    const { navigate } = useNavigationStore.getState()
    navigate({ kind: 'nandi' })
    expect(useNavigationStore.getState().history).toHaveLength(0)
  })

  it('caps the history stack', () => {
    const { navigate } = useNavigationStore.getState()
    for (let i = 0; i < MAX_HISTORY + 10; i++) {
      navigate({ kind: 'agent', agentId: `a${i}` })
    }
    expect(useNavigationStore.getState().history).toHaveLength(MAX_HISTORY)
  })

  it('matches active views by identity, ignoring focus targets', () => {
    const current = {
      kind: 'agent',
      agentId: 'a1',
      focusMessageId: 'm',
    } as const
    expect(isActiveView(current, { kind: 'agent', agentId: 'a1' })).toBe(true)
    expect(isActiveView(current, { kind: 'agent', agentId: 'a2' })).toBe(false)
    expect(isActiveView(current, { kind: 'inbox' })).toBe(false)
  })

  it('matches member profiles and delegation threads by id', () => {
    const member = { kind: 'member-profile', instanceId: 'i1' } as const
    expect(isActiveView(member, member)).toBe(true)
    expect(
      isActiveView(member, { kind: 'member-profile', instanceId: 'i2' })
    ).toBe(false)
    const thread = { kind: 'delegation-thread', taskId: 't1' } as const
    expect(isActiveView(thread, thread)).toBe(true)
    expect(
      isActiveView(thread, { kind: 'delegation-thread', taskId: 't2' })
    ).toBe(false)
  })
})

describe('the seven pages', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('replaces the current view without a Back step', () => {
    useNavigationStore.setState({ current: { kind: 'inbox' }, history: [] })
    useNavigationStore.getState().navigate({ kind: 'history' })
    useNavigationStore
      .getState()
      .replace({ kind: 'history', filters: { status: 'failed' } })
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'history',
      filters: { status: 'failed' },
    })
    useNavigationStore.getState().goBack()
    expect(useNavigationStore.getState().current).toEqual({ kind: 'inbox' })
  })

  it('puts every other view under one item', () => {
    const views: [View, NavSection][] = [
      // The Inbox is under Nandi, and what opens from it too
      [{ kind: 'inbox' }, 'nandi'],
      [{ kind: 'request', requestId: 'r' }, 'nandi'],
      [{ kind: 'work-item', id: 'w' }, 'nandi'],
      [{ kind: 'delegation-thread', taskId: 't' }, 'nandi'],
      [{ kind: 'intake-thread', conversationId: 'intake:q' }, 'nandi'],
      // History: past runs and decisions
      [{ kind: 'decisions' }, 'history'],
      [{ kind: 'agent', agentId: 'a' }, 'agents'],
      [{ kind: 'run', agentId: 'a', runId: 'r' }, 'agents'],
      [{ kind: 'run-detail', runId: 'r' }, 'history'],
      // Nandi's page and runs are its own item, not Agents'
      [{ kind: 'agent', agentId: DEFAULT_AGENT, tab: 'settings' }, 'nandi'],
      [{ kind: 'run', agentId: DEFAULT_AGENT, runId: 'r' }, 'nandi'],
      // Nandi keeps My notes
      [{ kind: 'notes', note: 'n' }, 'nandi'],
      [{ kind: 'workflow', workflowId: 'w' }, 'automations'],
      [{ kind: 'workflow-builder', workflowId: 'w' }, 'automations'],
      [{ kind: 'trigger', triggerId: 't' }, 'automations'],
      [{ kind: 'capability-detail', id: 'c' }, 'library'],
      [{ kind: 'starter-packs' }, 'library'],
      [{ kind: 'member-profile', instanceId: 'i' }, 'network'],
      // Studio's editor is reached from an agent's page
      [{ kind: 'studio', conversationId: 'c' }, 'agents'],
      // One place to talk: the home, its chats and live work
      [{ kind: 'nandi', conversationId: 'chat:c' }, 'nandi'],
      [{ kind: 'live-run', runId: 'r' }, 'nandi'],
      [{ kind: 'settings', section: 'logs', logTab: 'egress' }, 'settings'],
    ]
    for (const [view, section] of views) {
      expect(navSection(view)).toBe(section)
    }
    expect(navSection({ kind: 'onboarding' })).toBeNull()
  })

  it('opens each item on its own page', () => {
    for (const section of NAV_SECTIONS) {
      expect(navSection(SECTION_VIEWS[section])).toBe(section)
    }
  })
})

describe('parseView', () => {
  it('accepts known view kinds as they are', () => {
    expect(
      parseView({ kind: 'network', tab: 'sharing', sharing: 'callers' })
    ).toEqual({ kind: 'network', tab: 'sharing', sharing: 'callers' })
    expect(parseView({ kind: 'run-detail', runId: 'r1' })).toEqual({
      kind: 'run-detail',
      runId: 'r1',
    })
    expect(parseView({ kind: 'library', tab: 'pilots' })).toEqual({
      kind: 'library',
      tab: 'pilots',
    })
  })

  it('accepts the local-identity views', () => {
    for (const view of [
      { kind: 'onboarding' },
      { kind: 'network' },
      { kind: 'member-profile', instanceId: 'i1' },
      { kind: 'settings', section: 'account' },
      { kind: 'delegation-thread', taskId: 't1' },
    ]) {
      expect(parseView(view)).toEqual(view)
    }
  })

  it('defaults Studio to the conversation list', () => {
    expect(parseView({ kind: 'studio' })).toEqual({
      kind: 'studio',
      conversationId: null,
    })
  })

  it('rejects anything else', () => {
    for (const value of [
      null,
      'dashboard',
      {},
      { kind: 'nope' },
      { kind: 1 },
    ]) {
      expect(parseView(value)).toBeNull()
    }
    expect(parseView({ kind: 'toString' })).toBeNull()
  })
})
