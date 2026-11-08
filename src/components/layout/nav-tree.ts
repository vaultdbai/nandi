/**
 * The sidebar's tree: the seven pages, and one level under three of them:
 * Ask's latest chats, every agent, and Automations' workflows and
 * triggers. Home, Library, Network and Settings are single items (their
 * pages have their own tabs and sections).
 * Pure: the sidebar passes in the lists and renders what comes back.
 *
 * - Two levels, never more.
 * - Long lists show the newest or busiest few, then "All …".
 * - A node opens a view.
 */
import type {
  AgentInfo,
  AgentStatus,
  StudioConversationSummary,
} from '@/lib/bindings'
import type { SettingsSection } from '@/lib/settings-nav'
import {
  NAV_SECTIONS,
  SECTION_VIEWS,
  type NavSection,
  type View,
} from '@/store/navigation-store'

/** Agents listed before "All agents" */
export const LIST_LIMIT = 8
/** Ask chats listed before "All chats" */
export const CHAT_LIMIT = 5

/** A label to translate (`key`, with `values`), or a name as it is */
export type NavLabel =
  { key: string; values?: Record<string, string | number> } | { text: string }

export type NavDot = 'ok' | 'busy' | 'waiting' | 'error' | 'attention'

export interface NavNode {
  /** Unique in the tree; also what expansion is remembered by */
  id: string
  label: NavLabel
  view?: View
  /** A count on the right (Home's Inbox) */
  badge?: number
  dot?: NavDot
  /** "All chats": quieter than the list */
  subtle?: boolean
  /** "New chat": an action, set apart from the list it adds to */
  create?: boolean
  children?: NavNode[]
}

export interface NavTreeInput {
  inboxCount: number
  agents: readonly AgentInfo[]
  chats: readonly StudioConversationSummary[]
  /** Settings sections that need attention */
  attention: Partial<Record<SettingsSection, boolean>>
}

const STATUS_DOTS: Record<AgentStatus, NavDot> = {
  running: 'busy',
  waitingApproval: 'waiting',
  error: 'error',
  idle: 'ok',
}

/** Busy agents first (waiting, running, broken), then by name */
const STATUS_ORDER: Record<AgentStatus, number> = {
  waitingApproval: 0,
  running: 1,
  error: 2,
  idle: 3,
}

const key = (k: string, values?: Record<string, string | number>): NavLabel =>
  values ? { key: k, values } : { key: k }

/** New chat, the newest chats, All chats */
function chatNodes(chats: readonly StudioConversationSummary[]): NavNode[] {
  const recent = [...chats]
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, CHAT_LIMIT)
  return [
    {
      id: 'studio:new',
      label: key('nav.newChat'),
      // Empty until its first message is sent, which creates the chat
      view: { kind: 'studio', conversationId: null, draft: true },
      create: true,
    },
    ...recent.map(chat => ({
      id: `chat:${chat.id}`,
      label: chat.title ? { text: chat.title } : key('studio.untitled'),
      view: { kind: 'studio', conversationId: chat.id } as View,
    })),
    {
      id: 'studio:all',
      label: key('nav.tree.allChats', { count: chats.length }),
      view: { kind: 'studio', conversationId: null },
      subtle: true,
    },
  ]
}

/**
 * The built-in agents (Chief of Staff, Browser) on top, always shown; then every
 * other agent, busiest first; past the limit, All agents
 */
function agentNodes(agents: readonly AgentInfo[]): NavNode[] {
  const builtin = agents.filter(agent => agent.builtin)
  const sorted = agents
    .filter(agent => !agent.builtin)
    .sort(
      (a, b) =>
        STATUS_ORDER[a.status] - STATUS_ORDER[b.status] ||
        a.name.localeCompare(b.name)
    )
  const nodes: NavNode[] = [...builtin, ...sorted.slice(0, LIST_LIMIT)].map(
    agent => ({
      id: `agent:${agent.id}`,
      label: { text: agent.name || agent.id },
      view: { kind: 'agent', agentId: agent.id },
      dot: STATUS_DOTS[agent.status],
    })
  )
  if (sorted.length > LIST_LIMIT) {
    nodes.push({
      id: 'agents:all',
      label: key('nav.tree.allAgents', { count: agents.length }),
      view: { kind: 'agents' },
      subtle: true,
    })
  }
  return nodes
}

const tab = (id: string, label: string, view: View): NavNode => ({
  id,
  label: key(label),
  view,
})

/** The whole tree, in sidebar order */
export function buildNavTree(input: NavTreeInput): NavNode[] {
  const children: Record<NavSection, NavNode[] | undefined> = {
    // Home stays one item
    home: undefined,
    studio: chatNodes(input.chats),
    agents: agentNodes(input.agents),
    automations: [
      tab('automations:workflows', 'nav.workflows', {
        kind: 'automations',
        tab: 'workflows',
      }),
      tab('automations:triggers', 'nav.triggers', {
        kind: 'automations',
        tab: 'triggers',
      }),
    ],
    // Their pages' own tabs and sections are enough
    library: undefined,
    network: undefined,
    notes: undefined,
    settings: undefined,
  }
  return NAV_SECTIONS.map(section => ({
    id: section,
    label: key(`nav.${section}`),
    view: SECTION_VIEWS[section],
    badge:
      section === 'home' && input.inboxCount > 0 ? input.inboxCount : undefined,
    dot:
      section === 'settings' && Object.values(input.attention).some(Boolean)
        ? 'attention'
        : undefined,
    children: children[section],
  }))
}

/**
 * Whether `current` is `view` or within it: the same kind, and every
 * field `view` sets is the same (`{agent x}` holds `{agent x, tab runs}`)
 */
export function viewContains(view: View, current: View): boolean {
  const a = view as Record<string, unknown>
  const b = current as Record<string, unknown>
  if (a.kind !== b.kind) return false
  return Object.entries(a).every(
    ([field, value]) =>
      value === undefined || JSON.stringify(b[field]) === JSON.stringify(value)
  )
}

/**
 * The path to the node for `current`: the deepest node whose view holds
 * it, and its ancestors. Empty when nothing does. "All …" rows open their
 * page's own view, so they never stand for it: the page does.
 */
export function activePath(tree: readonly NavNode[], current: View): string[] {
  let best: string[] = []
  const walk = (nodes: readonly NavNode[], path: string[]) => {
    for (const node of nodes) {
      const here = [...path, node.id]
      if (
        node.view &&
        !node.subtle &&
        viewContains(node.view, current) &&
        here.length > best.length
      ) {
        best = here
      }
      if (node.children) walk(node.children, here)
    }
  }
  walk(tree, [])
  return best
}

/** The ids in the order the keyboard walks them: open branches only */
export function visibleIds(
  tree: readonly NavNode[],
  expanded: ReadonlySet<string>
): string[] {
  const ids: string[] = []
  const walk = (nodes: readonly NavNode[]) => {
    for (const node of nodes) {
      ids.push(node.id)
      if (node.children?.length && expanded.has(node.id)) walk(node.children)
    }
  }
  walk(tree)
  return ids
}

/** `id`'s parent, or null at the top */
export function parentOf(tree: readonly NavNode[], id: string): string | null {
  let found: string | null = null
  const walk = (nodes: readonly NavNode[], parent: string | null) => {
    for (const node of nodes) {
      if (node.id === id) found = parent
      if (node.children) walk(node.children, node.id)
    }
  }
  walk(tree, null)
  return found
}

export function findNode(tree: readonly NavNode[], id: string): NavNode | null {
  for (const node of tree) {
    if (node.id === id) return node
    const inner = node.children ? findNode(node.children, id) : null
    if (inner) return inner
  }
  return null
}
