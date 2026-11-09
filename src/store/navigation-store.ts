import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import type { ActivityFilters } from '@/components/home/activity-model'
import type { AgentTab } from '@/components/agents/agent-page-model'
import type { SettingsSection } from '@/lib/settings-nav'

/**
 * Everything the main pane can show. Navigation is plain state (no router):
 * a desktop app has no URLs to keep in sync.
 *
 * The sidebar has eight pages (`NAV_SECTIONS`); every other view is a
 * detail page that belongs to one of them (`navSection`).
 */
export type View =
  // The eight sidebar pages
  /** Inbox, Running, Recent; `welcome` greets the packs just installed */
  | { kind: 'home'; welcome?: { packIds: string[] } }
  /** Home › All activity: runs and delegations, `filters` kept for Back */
  | { kind: 'home-activity'; filters?: ActivityFilters }
  /** Home › Inbox: everything waiting on you, each with its fix */
  | { kind: 'inbox' }
  /** Home › Decisions: every decision made on the Inbox, by month */
  | { kind: 'decisions' }
  | { kind: 'agents' }
  | { kind: 'automations'; tab?: AutomationsTab }
  /**
   * Installed · Browse · Shared with me; `filter` is the kind chip,
   * `capability` the Can do filter, `item` the open drawer (`<type>:<id>`)
   */
  | {
      kind: 'library'
      tab?: LibraryTab
      filter?: LibraryKindFilter
      capability?: string
      item?: string
    }
  /**
   * People (directory), Teams, Groups (`group` opens one), Sharing
   * (`sharing` picks its view), Connection
   */
  | {
      kind: 'network'
      tab?: NetworkTab
      sharing?: SharingTab
      /** People filtered to one team's roster (or `MANAGE_TEAMS`) */
      team?: string
      /** Groups: the group open (its ID) */
      group?: string
      callers?: CallersView
    }
  /**
   * Settings, on one section (default: the last used, else Models).
   * `focus` highlights one item; `agentId` filters Internet access.
   */
  | {
      kind: 'settings'
      section?: SettingsSection
      focus?: string
      logTab?: AuditLogTab
      agentId?: string
    }
  // Detail pages
  /**
   * One agent's page (Overview by default). `focusMessageId` opens Chat
   * scrolled to (and highlighting) one message
   */
  | {
      kind: 'agent'
      agentId: string
      focusMessageId?: string
      tab?: AgentTab
    }
  /** A workflow's page (Overview by default) */
  | { kind: 'workflow'; workflowId: string; tab?: WorkflowTab }
  | { kind: 'trigger'; triggerId: string }
  /**
   * Ask: a conversation, the list (`null`), or with `draft` a new chat
   * that's created when its first message is sent
   */
  | { kind: 'studio'; conversationId: string | null; draft?: boolean }
  /** A historical run of an agent */
  | { kind: 'run'; agentId: string; runId: string }
  // First-launch setup
  /** Name, relay, and optional remote (local identity) */
  | { kind: 'onboarding' }
  /** First launch: pick curated packs to install */
  | { kind: 'starter-packs' }
  /** The user's notes, which Chief of Staff keeps; `note` is the one open */
  | { kind: 'notes'; note?: string }
  /** `tab` opens one tab (default: Timeline while active, else Overview) */
  | { kind: 'run-detail'; runId: string; tab?: RunDetailTab }
  | { kind: 'workflow-builder'; workflowId: string }
  | { kind: 'member-profile'; instanceId: string }
  | { kind: 'delegation-thread'; taskId: string }
  | { kind: 'capability-detail'; id: string }
  /** One workflow run as a project */
  | { kind: 'work-item'; id: string }
  /** One Request with its Steps (any of its task IDs); `stepId` opens a step */
  | { kind: 'request'; requestId: string; stepId?: string }
  /** One intake item's Chief of Staff conversation (`intake:<id>`) */
  | { kind: 'intake-thread'; conversationId: string }
  /** One Inbox item's Ask chat with the Chief of Staff (`inbox:<kind>:<id>`) */
  | { kind: 'inbox-thread'; conversationId: string }

export const RUN_DETAIL_TABS = [
  'overview',
  'summary',
  'timeline',
  'outputs',
  'files',
  'logs',
  'approvals',
] as const
export type RunDetailTab = (typeof RUN_DETAIL_TABS)[number]

export type AuditLogTab =
  'llm' | 'egress' | 'triggers' | 'pilots' | 'connectors' | 'decisions'

export type AutomationsTab = 'workflows' | 'triggers'
export type WorkflowTab = 'overview' | 'graph' | 'runs' | 'configuration'
export type LibraryTab = 'installed' | 'browse' | 'shared'
export type LibraryKindFilter =
  'all' | 'agents' | 'workflows' | 'tools' | 'packs'
export type NetworkTab = 'people' | 'groups' | 'sharing' | 'connection'
/** What you offer · Who may call you · Incoming requests · Shared · Capabilities */
export type SharingTab =
  'offer' | 'callers' | 'incoming' | 'shared' | 'capabilities'
/** Who may call you, as a list or the trust graph */
export type CallersView = 'list' | 'graph'
/** People's team filter: a team's roster, or managing teams */
export const MANAGE_TEAMS = '__manage'

/** The eight sidebar items, in order */
export const NAV_SECTIONS = [
  'home',
  'studio',
  'notes',
  'agents',
  'automations',
  'library',
  'network',
  'settings',
] as const

export type NavSection = (typeof NAV_SECTIONS)[number]

/** Which sidebar item a view belongs to (`null`: none, e.g. onboarding) */
export function navSection(view: View): NavSection | null {
  switch (view.kind) {
    case 'home':
    case 'home-activity':
    case 'inbox':
    case 'decisions':
    case 'request':
    case 'work-item':
    case 'delegation-thread':
    case 'intake-thread':
    case 'inbox-thread':
      return 'home'
    case 'agents':
    case 'agent':
    case 'run':
    case 'run-detail':
      return 'agents'
    case 'automations':
    case 'workflow':
    case 'workflow-builder':
    case 'trigger':
      return 'automations'
    case 'library':
    case 'capability-detail':
    case 'starter-packs':
      return 'library'
    case 'network':
    case 'member-profile':
      return 'network'
    case 'studio':
      return 'studio'
    case 'notes':
      return 'notes'
    case 'settings':
      return 'settings'
    default:
      return null
  }
}

/** The view each sidebar item opens */
export const SECTION_VIEWS: Record<NavSection, View> = {
  home: { kind: 'home' },
  agents: { kind: 'agents' },
  automations: { kind: 'automations' },
  library: { kind: 'library' },
  network: { kind: 'network' },
  studio: { kind: 'studio', conversationId: null },
  notes: { kind: 'notes' },
  settings: { kind: 'settings' },
}

/** Bounded so a long session can't grow the back stack forever */
export const MAX_HISTORY = 50

/** A move held back by the leave guard until the user decides */
export type PendingLeave =
  | { mode: 'navigate'; view: View }
  | { mode: 'replace'; view: View }
  | { mode: 'back' }

interface NavigationState {
  current: View
  /** Previous views, most recent last */
  history: View[]
  /**
   * Set while the current view has unsaved changes (e.g. a workflow
   * draft): leaving asks first. Cleared on leave.
   */
  leaveGuarded: boolean
  /** The move waiting on "Discard changes?" */
  pendingLeave: PendingLeave | null

  navigate: (view: View) => void
  /** Swaps the current view without a Back step (e.g. changed filters) */
  replace: (view: View) => void
  goBack: () => void
  setLeaveGuarded: (guarded: boolean) => void
  /** Discards the unsaved changes and makes the pending move */
  confirmLeave: () => void
  /** Stays on the current view */
  cancelLeave: () => void
}

const sameView = (a: View, b: View) => JSON.stringify(a) === JSON.stringify(b)

export const useNavigationStore = create<NavigationState>()(
  devtools(
    (set, get) => {
      /** Holds a move that would leave a guarded view; `true` if held */
      const hold = (move: PendingLeave): boolean => {
        const { leaveGuarded, current } = get()
        if (!leaveGuarded) return false
        if (move.mode !== 'back' && sameView(current, move.view)) return false
        set({ pendingLeave: move }, undefined, 'leaveGuard/hold')
        return true
      }

      const navigateNow = (view: View) =>
        set(
          state =>
            sameView(state.current, view)
              ? state
              : {
                  current: view,
                  history: [...state.history, state.current].slice(
                    -MAX_HISTORY
                  ),
                  leaveGuarded: false,
                },
          undefined,
          `navigate/${view.kind}`
        )

      const replaceNow = (view: View) =>
        set(
          { current: view, leaveGuarded: false },
          undefined,
          `replace/${view.kind}`
        )

      const goBackNow = () =>
        set(
          state => {
            const previous = state.history.at(-1)
            if (!previous) return state
            return {
              current: previous,
              history: state.history.slice(0, -1),
              leaveGuarded: false,
            }
          },
          undefined,
          'goBack'
        )

      return {
        current: { kind: 'home' },
        history: [],
        leaveGuarded: false,
        pendingLeave: null,

        navigate: view => {
          if (!hold({ mode: 'navigate', view })) navigateNow(view)
        },

        replace: view => {
          if (!hold({ mode: 'replace', view })) replaceNow(view)
        },

        goBack: () => {
          if (get().history.length > 0 && hold({ mode: 'back' })) return
          goBackNow()
        },

        setLeaveGuarded: guarded =>
          set({ leaveGuarded: guarded }, undefined, 'leaveGuard/set'),

        confirmLeave: () => {
          const pending = get().pendingLeave
          set(
            { pendingLeave: null, leaveGuarded: false },
            undefined,
            'leaveGuard/confirm'
          )
          if (!pending) return
          if (pending.mode === 'navigate') navigateNow(pending.view)
          else if (pending.mode === 'replace') replaceNow(pending.view)
          else goBackNow()
        },

        cancelLeave: () =>
          set({ pendingLeave: null }, undefined, 'leaveGuard/cancel'),
      }
    },
    { name: 'navigation-store' }
  )
)

/** The view kinds a palette `navigate` may open (exhaustive by type) */
const VIEW_KINDS: Record<View['kind'], true> = {
  home: true,
  'home-activity': true,
  inbox: true,
  decisions: true,
  agents: true,
  automations: true,
  library: true,
  settings: true,
  agent: true,
  workflow: true,
  trigger: true,
  studio: true,
  run: true,
  onboarding: true,
  'starter-packs': true,
  notes: true,
  'run-detail': true,
  'workflow-builder': true,
  network: true,
  'member-profile': true,
  'delegation-thread': true,
  'capability-detail': true,
  'work-item': true,
  request: true,
  'intake-thread': true,
  'inbox-thread': true,
}

/**
 * A `View` from untyped JSON (e.g. a palette result), or `null` if its
 * `kind` isn't one the main pane knows. Fields are trusted as-is.
 */
export function parseView(value: unknown): View | null {
  if (typeof value !== 'object' || value === null) return null
  const kind = (value as { kind?: unknown }).kind
  if (typeof kind !== 'string' || !Object.hasOwn(VIEW_KINDS, kind)) {
    return null
  }
  const view = value as View
  // Studio's conversation is required by the type; default to the list
  if (view.kind === 'studio' && view.conversationId === undefined) {
    return { kind: 'studio', conversationId: null }
  }
  return view
}

/** Whether a sidebar item for `target` should render as active */
export function isActiveView(current: View, target: View): boolean {
  if (current.kind !== target.kind) return false
  switch (target.kind) {
    case 'agent':
      return current.kind === 'agent' && current.agentId === target.agentId
    case 'workflow':
      return (
        current.kind === 'workflow' && current.workflowId === target.workflowId
      )
    case 'trigger':
      return (
        current.kind === 'trigger' && current.triggerId === target.triggerId
      )
    case 'studio':
      return (
        current.kind === 'studio' &&
        current.conversationId === target.conversationId
      )
    case 'run':
      return current.kind === 'run' && current.runId === target.runId
    case 'member-profile':
      return (
        current.kind === 'member-profile' &&
        current.instanceId === target.instanceId
      )
    case 'delegation-thread':
      return (
        current.kind === 'delegation-thread' && current.taskId === target.taskId
      )
    case 'work-item':
      return current.kind === 'work-item' && current.id === target.id
    case 'request':
      return (
        current.kind === 'request' && current.requestId === target.requestId
      )
    case 'intake-thread':
      return (
        current.kind === 'intake-thread' &&
        current.conversationId === target.conversationId
      )
    case 'inbox-thread':
      return (
        current.kind === 'inbox-thread' &&
        current.conversationId === target.conversationId
      )
    default:
      return true
  }
}
