import { describe, expect, it } from 'vitest'
import type { AgentInfo, StudioConversationSummary } from '@/lib/bindings'
import {
  activePath,
  buildNavTree,
  CHAT_LIMIT,
  findNode,
  LIST_LIMIT,
  parentOf,
  viewContains,
  visibleIds,
  type NavNode,
  type NavTreeInput,
} from './nav-tree'

const agent = (
  id: string,
  status: AgentInfo['status'] = 'idle'
): AgentInfo => ({
  id,
  name: (id[0]?.toUpperCase() ?? '') + id.slice(1),
  description: '',
  skillPath: '',
  status,
  currentTask: null,
  origin: { kind: 'local' },
  builtin: false,
})

const chat = (id: string, updatedAt: number): StudioConversationSummary => ({
  id,
  title: `Chat ${id}`,
  createdAt: 0,
  updatedAt,
  lastMessage: null,
})

const input = (overrides: Partial<NavTreeInput> = {}): NavTreeInput => ({
  inboxCount: 0,
  agents: [],
  chats: [],
  attention: {},
  ...overrides,
})

const ids = (nodes: NavNode[] | undefined) => nodes?.map(n => n.id)

describe('buildNavTree', () => {
  it('keeps the eight pages, with Home as one item and its count', () => {
    const tree = buildNavTree(input({ inboxCount: 3 }))
    expect(ids(tree)).toEqual([
      'home',
      'studio',
      'notes',
      'agents',
      'automations',
      'library',
      'network',
      'settings',
    ])
    expect(tree[0]?.children).toBeUndefined()
    expect(tree[0]?.badge).toBe(3)
    expect(buildNavTree(input())[0]?.badge).toBeUndefined()
  })

  it('has New chat, the newest five chats and All chats under Ask', () => {
    const chats = Array.from({ length: CHAT_LIMIT + 2 }, (_, i) =>
      chat(`c${i}`, i)
    )
    const ask = findNode(buildNavTree(input({ chats })), 'studio')?.children
    expect(ids(ask)).toEqual([
      'studio:new',
      'chat:c6',
      'chat:c5',
      'chat:c4',
      'chat:c3',
      'chat:c2',
      'studio:all',
    ])
    // With no chats yet: New chat and All chats
    expect(ids(findNode(buildNavTree(input()), 'studio')?.children)).toEqual([
      'studio:new',
      'studio:all',
    ])
  })

  it('lists each agent, busy ones first, with nothing under it', () => {
    const agents = [
      agent('zed'),
      agent('amy'),
      agent('bob', 'running'),
      agent('cat', 'waitingApproval'),
      ...Array.from({ length: LIST_LIMIT }, (_, i) => agent(`x${i}`)),
    ]
    const list =
      findNode(buildNavTree(input({ agents })), 'agents')?.children ?? []
    expect(ids(list.slice(0, 3))).toEqual([
      'agent:cat',
      'agent:bob',
      'agent:amy',
    ])
    expect(list.slice(0, 2).map(n => n.dot)).toEqual(['waiting', 'busy'])
    expect(list.every(n => n.children === undefined)).toBe(true)
    expect(list).toHaveLength(LIST_LIMIT + 1)
    expect(list.at(-1)?.id).toBe('agents:all')
  })

  it('keeps the built-in agents on top, idle or not, past the limit', () => {
    const chief = { ...agent('chief-of-staff'), builtin: true }
    const agents = [
      chief,
      agent('amy', 'running'),
      ...Array.from({ length: LIST_LIMIT }, (_, i) => agent(`x${i}`)),
    ]
    const list =
      findNode(buildNavTree(input({ agents })), 'agents')?.children ?? []
    expect(ids(list.slice(0, 2))).toEqual(['agent:chief-of-staff', 'agent:amy'])
    expect(list).toHaveLength(LIST_LIMIT + 2)
    expect(list.at(-1)?.id).toBe('agents:all')
  })

  it('has Workflows and Triggers under Automations; Library, Network and Settings stand alone', () => {
    const tree = buildNavTree(input())
    expect(ids(findNode(tree, 'automations')?.children)).toEqual([
      'automations:workflows',
      'automations:triggers',
    ])
    for (const page of ['library', 'network', 'settings']) {
      expect(findNode(tree, page)?.children).toBeUndefined()
    }
  })

  it('sets New chat apart from the chats', () => {
    const ask = findNode(
      buildNavTree(input({ chats: [chat('c', 1)] })),
      'studio'
    )
    const [create, first] = ask?.children ?? []
    expect(create?.create).toBe(true)
    expect(first?.create).toBeUndefined()
  })

  it('dots Settings when a section needs attention', () => {
    expect(
      findNode(buildNavTree(input({ attention: { models: true } })), 'settings')
        ?.dot
    ).toBe('attention')
    expect(findNode(buildNavTree(input()), 'settings')?.dot).toBeUndefined()
  })

  it('is never more than two levels deep', () => {
    const tree = buildNavTree(
      input({ agents: [agent('amy')], chats: [chat('c', 1)] })
    )
    const depth = (nodes: NavNode[] | undefined): number =>
      nodes?.length ? 1 + Math.max(...nodes.map(n => depth(n.children))) : 0
    expect(depth(tree)).toBe(2)
  })
})

describe('the active node', () => {
  const tree = buildNavTree(
    input({ agents: [agent('amy')], chats: [chat('c1', 1)] })
  )

  it('is the deepest node holding the view', () => {
    expect(
      activePath(tree, { kind: 'agent', agentId: 'amy', tab: 'runs' })
    ).toEqual(['agents', 'agent:amy'])
    expect(
      activePath(tree, { kind: 'settings', section: 'logs', logTab: 'llm' })
    ).toEqual(['settings'])
    // The chat list is Ask itself, not its All chats row
    expect(activePath(tree, { kind: 'studio', conversationId: null })).toEqual([
      'studio',
    ])
    expect(activePath(tree, { kind: 'studio', conversationId: 'c1' })).toEqual([
      'studio',
      'chat:c1',
    ])
  })

  it('is nothing for views outside the tree', () => {
    expect(activePath(tree, { kind: 'onboarding' })).toEqual([])
    // An agent not in the list lights nothing below Agents
    expect(activePath(tree, { kind: 'agent', agentId: 'zed' })).toEqual([])
  })

  it('compares only the fields the node sets', () => {
    expect(
      viewContains(
        { kind: 'library' },
        { kind: 'library', tab: 'browse', filter: 'tools' }
      )
    ).toBe(true)
    expect(
      viewContains(
        { kind: 'library', tab: 'installed' },
        { kind: 'library', tab: 'browse' }
      )
    ).toBe(false)
  })
})

describe('keyboard order', () => {
  const tree = buildNavTree(input({ agents: [agent('amy')] }))

  it('walks open branches only', () => {
    expect(visibleIds(tree, new Set())).toHaveLength(8)
    expect(visibleIds(tree, new Set(['agents'])).slice(3, 6)).toEqual([
      'agents',
      'agent:amy',
      'automations',
    ])
  })

  it('knows each node’s parent', () => {
    expect(parentOf(tree, 'agent:amy')).toBe('agents')
    expect(parentOf(tree, 'agents')).toBeNull()
  })
})
