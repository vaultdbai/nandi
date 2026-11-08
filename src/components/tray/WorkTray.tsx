import { useTranslation } from 'react-i18next'
import { ArrowRight, Check, PanelRightClose, X } from 'lucide-react'
import type { NavLabel } from '@/components/layout/nav-tree'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useHomeFeed } from '@/hooks/useHome'
import { decideApproval } from '@/lib/agent-actions'
import { cn } from '@/lib/utils'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { useUIStore } from '@/store/ui-store'
import {
  buildTray,
  type TrayGroup,
  type TrayGroupId,
  type TrayRow,
  type TrayTone,
} from './tray-model'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

const sameView = (a: View, b: View) => JSON.stringify(a) === JSON.stringify(b)

const TONE_DOTS: Record<TrayTone, string> = {
  needsYou: 'bg-warning',
  working: 'bg-info animate-pulse',
  done: 'bg-success',
  failed: 'bg-destructive',
  quiet: 'bg-muted-foreground/50',
}

const GROUP_TEXT: Record<TrayGroupId, string> = {
  needsYou: 'text-warning',
  working: 'text-info',
  done: 'text-success',
}

/**
 * The work tray, on the right of every page: what needs you, what's
 * working, and what finished today, one small line each with a colour for
 * its state. A row opens its page; a tool approval can be decided in place.
 * Each group links to All. Hidden with the title bar's toggle or Ctrl+2.
 */
export function WorkTray() {
  const { t } = useTranslation()
  const home = useHomeFeed()
  const current = useNavigationStore(state => state.current)
  const groups = buildTray({
    inbox: home.inbox.items,
    running: home.running,
    recent: home.recent,
  })

  return (
    <aside
      aria-label={t('tray.title')}
      className="flex h-full min-h-0 flex-col border-s bg-background"
      data-testid="work-tray"
    >
      <div className="flex h-9 shrink-0 items-center justify-between gap-2 border-b ps-3 pe-1">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          {t('tray.title')}
        </h2>
        <Button
          variant="ghost"
          size="icon"
          className="size-6 text-muted-foreground"
          title={t('titlebar.hideRightSidebar')}
          aria-label={t('titlebar.hideRightSidebar')}
          onClick={() => useUIStore.getState().setRightSidebarVisible(false)}
        >
          <PanelRightClose className="size-3.5 rtl:rotate-180" />
        </Button>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-4 p-2">
          {groups.map(group => (
            <Group key={group.id} group={group} current={current} />
          ))}
        </div>
      </ScrollArea>
    </aside>
  )
}

function useLabel(): (label: NavLabel) => string {
  const { t } = useTranslation()
  return label => ('text' in label ? label.text : t(label.key, label.values))
}

function Group({ group, current }: { group: TrayGroup; current: View }) {
  const { t } = useTranslation()
  const title = t(`tray.group.${group.id}`)
  return (
    <section aria-label={title} data-testid={`tray-${group.id}`}>
      <div className="flex items-center gap-1.5 px-1 pb-1">
        <span
          className={cn(
            'text-[11px] font-semibold tracking-wide uppercase',
            group.total > 0 ? GROUP_TEXT[group.id] : 'text-muted-foreground'
          )}
        >
          {title}
        </span>
        {group.total > 0 && (
          <span className="text-[11px] text-muted-foreground tabular-nums">
            {group.total}
          </span>
        )}
        <button
          type="button"
          className="ms-auto flex items-center gap-0.5 rounded px-1 text-[11px] text-muted-foreground hover:text-foreground"
          onClick={() => navigate(group.all)}
        >
          {t('tray.all')}
          <ArrowRight aria-hidden className="size-3 rtl:rotate-180" />
        </button>
      </div>
      {group.rows.length === 0 ? (
        <p className="px-1 text-[11px] text-muted-foreground/70">
          {t(`tray.empty.${group.id}`)}
        </p>
      ) : (
        <ul className="flex flex-col">
          {group.rows.map(row => (
            <li key={row.id}>
              <Row
                row={row}
                active={sameView(row.view, current)}
                muted={group.id === 'done'}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function Row({
  row,
  active,
  muted,
}: {
  row: TrayRow
  active: boolean
  /** Finished work reads quieter than live work */
  muted: boolean
}) {
  const { t } = useTranslation()
  const label = useLabel()
  const text = label(row.label)
  return (
    <div
      className={cn(
        'group/row flex items-center gap-1 rounded-md hover:bg-accent/50',
        active && 'bg-accent'
      )}
    >
      <button
        type="button"
        title={text}
        onClick={() => navigate(row.view)}
        className="flex min-w-0 flex-1 items-center gap-2 px-1.5 py-1 text-start"
      >
        <span
          aria-hidden
          className={cn('size-1.5 shrink-0 rounded-full', TONE_DOTS[row.tone])}
        />
        <span className="sr-only">{t(`tray.tone.${row.tone}`)}</span>
        <span
          className={cn(
            'min-w-0 flex-1 truncate text-xs',
            muted && 'text-muted-foreground',
            row.tone === 'failed' && 'text-destructive'
          )}
        >
          {text}
        </span>
        {row.meta && (
          <span className="max-w-[40%] shrink-0 truncate text-[11px] text-muted-foreground tabular-nums">
            {label(row.meta)}
          </span>
        )}
      </button>
      {row.approval && <ApprovalButtons approval={row.approval} />}
    </div>
  )
}

/** Approve or reject a tool call without leaving the page */
function ApprovalButtons({
  approval,
}: {
  approval: NonNullable<TrayRow['approval']>
}) {
  const { t } = useTranslation()
  const decide = (decision: 'approved' | 'rejected') =>
    void decideApproval(approval.agentId, approval.approvalId, decision)
  return (
    <span className="flex shrink-0 items-center pe-1">
      <Button
        variant="ghost"
        size="icon"
        className="size-5 text-success"
        title={t('tray.approve')}
        aria-label={t('tray.approve')}
        onClick={() => decide('approved')}
      >
        <Check className="size-3" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        className="size-5 text-destructive"
        title={t('tray.reject')}
        aria-label={t('tray.reject')}
        onClick={() => decide('rejected')}
      >
        <X className="size-3" />
      </Button>
    </span>
  )
}
