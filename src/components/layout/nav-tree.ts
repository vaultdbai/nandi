/**
 * The sidebar's tree: the seven pages, and one level under four of them:
 * Home's Inbox, Decisions and All activity, Nandi's live work, latest chats
 * and My notes, the person's agents, and Automations' workflows and
 * triggers. Library, Network and Settings are single items (their pages
 * have their own tabs and sections).
 * Pure: the sidebar passes in the lists and renders what comes back.
 *
 * - Two levels, never more.
 * - Long lists show the newest or busiest few, then "All …".
 * - A node opens a view.
 */
import type {
  AgentInfo,
  AgentStatus,
  ChatSummary,
  RunSummary,
} from '@/lib/bindings'
import { chatLabel } from '@/components/nandi/nandi-home-model'
import {
  defaultAgentName,
  isDefaultAgent,
  ownAgents,
} from '@/lib/agent-identity'
import type { SettingsSection } from '@/lib/settings-nav'
import {
  BOTTOM_SECTION,
  NAV_SECTIONS,
  SECTION_VIEWS,
  type NavSection,
  type View,
} from '@/store/navigation-store'

/** Agents listed before "All agents" */
export const LIST_LIMIT = 8
/** Chats listed under Nandi */
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
  /** A count on the right (Home and its Inbox) */
  badge?: number
  /** Where the count itself goes when clicked (Home's: the Inbox page) */
  badgeView?: View
  dot?: NavDot
  /** "All chats": quieter than the list */
  subtle?: boolean
  /** "New chat": an action, set apart from the list it adds to */
  create?: boolean
  /** The first of the items at the bottom (Library, Network, Settings) */
  apart?: boolean
  children?: NavNode[]
}

export interface NavTreeInput {
  inboxCount: number
  agents: readonly AgentInfo[]
  /** The person's chats with their agents (`list_chats`) */
  chats: readonly ChatSummary[]
  /** Live work, newest first (`list_live_runs`) */
  live?: readonly RunSummary[]
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

/**
 * Live work, what waits on the person first, named for its agent:
 * finished work stays on Ask's page only
 */
function liveNodes(
  live: readonly RunSummary[],
  agents: readonly AgentInfo[]
): NavNode[] {
  const waiting = live.filter(run => run.status === 'awaitingApproval')
  const running = live.filter(run => run.status === 'running')
  return [...waiting, ...running].slice(0, CHAT_LIMIT).map(run => ({
    id: `live:${run.id}`,
    label: {
      text: agents.find(a => a.id === run.agentId)?.name || run.agentId,
    },
    view: { kind: 'live-run', runId: run.id },
    dot: run.status === 'awaitingApproval' ? 'waiting' : 'busy',
  }))
}

/** New chat, the live work, the newest chats, then My notes */
function nandiNodes(
  chats: readonly ChatSummary[],
  live: NavNode[],
  agents: readonly AgentInfo[],
  inboxCount: number
): NavNode[] {
  const recent = [...chats]
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, CHAT_LIMIT)
  return [
    // What waits on you and My notes first, right under Nandi
    {
      ...tab('nandi:inbox', 'nav.inbox', { kind: 'inbox' }),
      badge: inboxCount > 0 ? inboxCount : undefined,
    },
    tab('nandi:notes', 'nav.notes', { kind: 'notes' }),
    {
      id: 'nandi:new',
      label: key('nav.newChat'),
      // The home's composer: sending the first message makes the chat
      view: { kind: 'nandi' },
      create: true,
    },
    ...live,
    ...recent.map(chat => ({
      id: `chat:${chat.id}`,
      label: chatLabel(chat, agents),
      view: { kind: 'nandi', conversationId: chat.id } as View,
    })),
  ]
}

/**
 * The person's own agents, busiest first; past the limit, All agents with
 * their count. Nandi has its own item on top, so it is never one of them.
 */
function agentNodes(agents: readonly AgentInfo[]): NavNode[] {
  const own = ownAgents(agents).sort(
    (a, b) =>
      STATUS_ORDER[a.status] - STATUS_ORDER[b.status] ||
      a.name.localeCompare(b.name)
  )
  const nodes: NavNode[] = own.slice(0, LIST_LIMIT).map(agent => ({
    id: `agent:${agent.id}`,
    label: { text: agent.name || agent.id },
    view: { kind: 'agent', agentId: agent.id },
    dot: STATUS_DOTS[agent.status],
  }))
  if (own.length > LIST_LIMIT) {
    nodes.push({
      id: 'agents:all',
      label: key('nav.tree.allAgents', { count: own.length }),
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

/** Nandi's status, and Settings' "needs attention" */
function sectionDot(
  section: NavSection,
  input: NavTreeInput,
  nandi: AgentInfo | undefined
): NavDot | undefined {
  if (section === 'nandi') return nandi && STATUS_DOTS[nandi.status]
  if (section === 'settings' && Object.values(input.attention).some(Boolean)) {
    return 'attention'
  }
  return undefined
}

/** The whole tree, in sidebar order */
export function buildNavTree(input: NavTreeInput): NavNode[] {
  const children: Record<NavSection, NavNode[] | undefined> = {
    // Past decisions, and the audit log behind every run
    history: [
      tab('history:decisions', 'nav.tree.decisions', { kind: 'decisions' }),
      tab('history:audit', 'nav.auditLog', {
        kind: 'settings',
        section: 'logs',
      }),
    ],
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
    // One place to talk: its chats, live work and My notes
    nandi: nandiNodes(
      input.chats,
      liveNodes(input.live ?? [], input.agents),
      input.agents,
      input.inboxCount
    ),
    // Their pages' own tabs and sections are enough
    library: undefined,
    network: undefined,
    settings: undefined,
  }
  const nandi = input.agents.find(isDefaultAgent)
  return NAV_SECTIONS.map(section => ({
    id: section,
    // Nandi goes by its own name, which the person may change
    label:
      section === 'nandi'
        ? { text: defaultAgentName(input.agents) }
        : key(`nav.${section}`),
    view: SECTION_VIEWS[section],
    // The Inbox's count, on Nandi too while its branch is closed; it opens
    // the Inbox
    badge:
      section === 'nandi' && input.inboxCount > 0
        ? input.inboxCount
        : undefined,
    badgeView: section === 'nandi' ? { kind: 'inbox' } : undefined,
    dot: sectionDot(section, input, nandi),
    apart: section === BOTTOM_SECTION || undefined,
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
        !node.create &&
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
