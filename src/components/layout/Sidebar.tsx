import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Bot,
  ChevronRight,
  Plus,
  House,
  Library,
  MessageSquare,
  Network,
  NotebookPen,
  Settings,
  Workflow,
} from 'lucide-react'
import { HealthDot } from '@/components/network/NetworkHelp'
import { useAgents } from '@/hooks/useAgents'
import { useInbox } from '@/hooks/useInbox'
import { useNetworkHealth } from '@/hooks/useNetworkHealth'
import { useSettingsAttention } from '@/hooks/useSettingsAttention'
import { cn } from '@/lib/utils'
import { useStudioConversations } from '@/store/agent-store'
import {
  navSection,
  useNavigationStore,
  type NavSection,
  type View,
} from '@/store/navigation-store'
import { useNavTreeStore } from '@/store/nav-tree-store'
import {
  activePath,
  buildNavTree,
  findNode,
  parentOf,
  visibleIds,
  type NavDot,
  type NavLabel,
  type NavNode,
} from './nav-tree'

const ICONS: Record<NavSection, React.ReactNode> = {
  home: <House />,
  agents: <Bot />,
  automations: <Workflow />,
  library: <Library />,
  network: <Network />,
  studio: <MessageSquare />,
  notes: <NotebookPen />,
  settings: <Settings />,
}

const DOT_COLORS: Record<NavDot, string> = {
  ok: 'bg-success',
  busy: 'bg-info animate-pulse',
  waiting: 'bg-warning',
  error: 'bg-destructive',
  attention: 'bg-warning',
}

/**
 * The eight pages as a tree (`nav-tree.ts`): under Home its Inbox and All
 * activity, under Ask the latest chats, under Agents each agent, under the
 * rest their tabs and sections. Clicking a name opens it and its branch;
 * the chevron only opens or closes. The arrow keys move through what's
 * open, Right and Left open and close, Enter opens. The branches leading
 * to the current page open by themselves, and the rest stay as you left
 * them (`nav-tree-store`).
 */
export function Sidebar() {
  const { t } = useTranslation()
  const listRef = useRef<HTMLUListElement>(null)
  const current = useNavigationStore(state => state.current)
  const expandedIds = useNavTreeStore(state => state.expanded)
  const expanded = new Set(expandedIds)

  const tree = buildNavTree({
    // What waits on you now (the Inbox); failures aren't counted
    inboxCount: useInbox().count,
    agents: useAgents().data ?? [],
    chats: useStudioConversations() ?? [],
    attention: useSettingsAttention(),
  })
  const path = activePath(tree, current)
  // Views without a node of their own (a run, a Request) light their page
  const section = navSection(current)
  const activeId = path.at(-1) ?? section

  // Open the branches above the current page
  const revealKey = path.slice(0, -1).join('/')
  useEffect(() => {
    if (revealKey) useNavTreeStore.getState().reveal(revealKey.split('/'))
  }, [revealKey])

  const open = (node: NavNode) => {
    if (node.children?.length) {
      useNavTreeStore.getState().setExpanded(node.id, true)
    }
    if (node.view) useNavigationStore.getState().navigate(node.view)
  }

  const focus = (id: string | null | undefined) => {
    if (!id) return
    listRef.current
      ?.querySelector<HTMLButtonElement>(
        `button[data-nav-item="${CSS.escape(id)}"]`
      )
      ?.focus()
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
    const id = (document.activeElement as HTMLElement | null)?.dataset.navItem
    if (!id) return
    const ids = visibleIds(tree, expanded)
    const index = ids.indexOf(id)
    const node = findNode(tree, id)
    const branch = Boolean(node?.children?.length)
    const store = useNavTreeStore.getState()
    switch (event.key) {
      case 'ArrowDown':
        focus(ids[(index + 1) % ids.length])
        break
      case 'ArrowUp':
        focus(ids[(index - 1 + ids.length) % ids.length])
        break
      case 'Home':
        focus(ids[0])
        break
      case 'End':
        focus(ids.at(-1))
        break
      case 'ArrowRight':
        if (!branch) return
        if (expanded.has(id)) focus(node?.children?.[0]?.id)
        else store.setExpanded(id, true)
        break
      case 'ArrowLeft':
        if (branch && expanded.has(id)) store.setExpanded(id, false)
        else focus(parentOf(tree, id))
        break
      default:
        return
    }
    event.preventDefault()
  }

  return (
    <nav aria-label={t('nav.label')} className="flex h-full min-h-0 flex-col">
      <ul
        ref={listRef}
        onKeyDown={onKeyDown}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-2"
      >
        {tree.map(node => (
          <NavItem
            key={node.id}
            node={node}
            level={0}
            expanded={expanded}
            activeId={activeId}
            activeSection={section}
            onOpen={open}
          />
        ))}
      </ul>
    </nav>
  )
}

function useLabel(): (label: NavLabel) => string {
  const { t } = useTranslation()
  return label => ('text' in label ? label.text : t(label.key, label.values))
}

function NavItem({
  node,
  level,
  expanded,
  activeId,
  activeSection,
  onOpen,
}: {
  node: NavNode
  level: number
  expanded: ReadonlySet<string>
  activeId: string | null
  activeSection: NavSection | null
  onOpen: (node: NavNode) => void
}) {
  const { t } = useTranslation()
  const labelOf = useLabel()
  const label = labelOf(node.label)
  const branch = Boolean(node.children?.length)
  const open = branch && expanded.has(node.id)
  const top = level === 0
  const current = node.id === activeId
  // The page you're under, while a child of it is the current one
  const within = top && node.id === activeSection && !current

  return (
    <li
      className={cn(
        'list-none',
        // New chat sits above the chats, with a line between
        node.create && 'mb-0.5 border-b border-border/60 pb-0.5'
      )}
    >
      <div
        className={cn(
          'group flex items-center rounded-lg text-sm text-foreground/90 transition-colors hover:bg-accent',
          current &&
            'bg-primary font-medium text-primary-foreground shadow-sm shadow-black/20 hover:bg-primary',
          within && 'font-medium text-foreground'
        )}
        style={{ paddingInlineStart: level * 12 }}
      >
        <span
          aria-hidden
          onClick={() => branch && useNavTreeStore.getState().toggle(node.id)}
          className={cn(
            'flex size-5 shrink-0 items-center justify-center rounded [&_svg]:size-3.5',
            branch ? 'cursor-pointer hover:bg-foreground/10' : 'invisible'
          )}
        >
          <ChevronRight
            className={cn('transition-transform', open && 'rotate-90')}
          />
        </span>
        <button
          type="button"
          data-nav-item={node.id}
          aria-current={current ? 'page' : undefined}
          aria-expanded={branch ? open : undefined}
          aria-label={
            node.badge === undefined || node.badgeView
              ? undefined
              : t('nav.withBadge', { label, count: node.badge })
          }
          onClick={() => onOpen(node)}
          className={cn(
            'flex min-w-0 flex-1 items-center gap-2 rounded-lg py-1.5 pe-2 text-start outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground',
            current && '[&_svg]:text-primary-foreground/90',
            !top && 'py-1 text-[13px]',
            node.subtle && !current && 'text-muted-foreground',
            node.create &&
              !current &&
              'font-medium text-primary [&_svg]:text-primary'
          )}
        >
          {top && <span aria-hidden>{ICONS[node.id as NavSection]}</span>}
          {node.create && <Plus aria-hidden className="!size-3.5" />}
          {node.dot && !top && (
            <span
              aria-hidden
              data-dot={node.dot}
              className={cn(
                'size-2 shrink-0 rounded-full',
                DOT_COLORS[node.dot]
              )}
            />
          )}
          <span className="min-w-0 flex-1 truncate">{label}</span>
          {node.badge !== undefined && !node.badgeView && (
            <span
              aria-hidden
              className="rounded-full bg-warning px-1.5 text-xs font-semibold text-black/80 tabular-nums"
            >
              {node.badge}
            </span>
          )}
          {node.dot && top && (
            <span
              aria-hidden
              data-dot={node.dot}
              className={cn(
                'size-2 shrink-0 rounded-full',
                DOT_COLORS[node.dot]
              )}
            />
          )}
          {node.id === 'network' && <NetworkDot />}
        </button>
        {node.badge !== undefined && node.badgeView && (
          <BadgeButton
            count={node.badge}
            view={node.badgeView}
            label={t('nav.withBadge', {
              label: t('nav.tree.inbox'),
              count: node.badge,
            })}
          />
        )}
      </div>
      {open && (
        <ul className="mt-0.5 flex flex-col gap-0.5">
          {node.children?.map(child => (
            <NavItem
              key={child.id}
              node={child}
              level={level + 1}
              expanded={expanded}
              activeId={activeId}
              activeSection={activeSection}
              onOpen={onOpen}
            />
          ))}
        </ul>
      )}
    </li>
  )
}

/** A count that opens its own page (Home's: the Inbox) */
function BadgeButton({
  count,
  view,
  label,
}: {
  count: number
  view: View
  label: string
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={() => useNavigationStore.getState().navigate(view)}
      className="me-2 rounded-full bg-warning px-1.5 text-xs font-semibold text-black/80 tabular-nums outline-none hover:brightness-95 focus-visible:ring-[3px] focus-visible:ring-ring/50"
    >
      {count}
    </button>
  )
}

/**
 * The connection at a glance (network.md §3): green when others can reach
 * this Nandi, amber when connecting or not listed yet, red when the relay
 * refused, nothing when no network is set. Never a count.
 */
function NetworkDot() {
  const { t } = useTranslation()
  const { health, label: key } = useNetworkHealth()
  if (health === 'none') return null
  const label = t(key)
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      data-testid="network-dot"
      data-health={health}
      className="inline-flex"
    >
      <HealthDot health={health} />
    </span>
  )
}
