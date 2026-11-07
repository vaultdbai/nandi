import { useTranslation } from 'react-i18next'
import { FilePen, FilePlus, FileX } from 'lucide-react'
import { useNow } from '@/components/agents/task-utils'
import { useRecentChanges } from '@/hooks/useLiveChanges'
import type { FileAction, LiveChange } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import {
  authorText,
  changeFiles,
  changeTitle,
  viewOf,
  whenText,
} from './live-model'
import { RefreshButton } from './RefreshButton'

/** How many changes Home shows */
const SHOWN = 8
const TICK_MS = 30_000

const ACTION_ICONS: Record<FileAction, React.ReactNode> = {
  created: <FilePlus aria-hidden className="size-4 text-success" />,
  updated: <FilePen aria-hidden className="size-4 text-info" />,
  deleted: <FileX aria-hidden className="size-4 text-muted-foreground" />,
}

/**
 * Recent changes on Home: notes, agents, workflows and files changed by
 * you, your agents, workflows or someone outside Nandi, each linking to
 * its page. Hidden until something changes.
 */
export function RecentChanges() {
  const { t } = useTranslation()
  const { data: changes = [] } = useRecentChanges()
  const now = useNow(true, TICK_MS)
  if (changes.length === 0) return null
  return (
    <section
      aria-label={t('live.recent.title')}
      className="flex flex-col gap-1"
      data-testid="recent-changes"
    >
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          {t('live.recent.title')}
        </h2>
        <RefreshButton />
      </div>
      <ul className="flex flex-col divide-y rounded-lg border">
        {changes.slice(0, SHOWN).map(change => (
          <li key={change.id}>
            <ChangeRow change={change} now={now} />
          </li>
        ))}
      </ul>
    </section>
  )
}

function ChangeRow({ change, now }: { change: LiveChange; now: number }) {
  const { t, i18n } = useTranslation()
  const view = viewOf(change)
  const files = changeFiles(change, t)
  return (
    <button
      type="button"
      disabled={!view}
      onClick={() => view && useNavigationStore.getState().navigate(view)}
      className={cn(
        'flex w-full items-center gap-3 px-3 py-2 text-start text-sm',
        view && 'hover:bg-accent'
      )}
    >
      {ACTION_ICONS[change.action]}
      <span className="min-w-0 flex-1">
        <span className="block truncate">
          {t(`live.action.${change.action}`, {
            what: changeTitle(change, t),
            area: t(`live.area.${change.area}`),
          })}
        </span>
        {files && (
          <span className="block truncate text-xs text-muted-foreground">
            {files}
          </span>
        )}
      </span>
      <span className="shrink-0 text-xs text-muted-foreground">
        {authorText(change.author, t)} ·{' '}
        {whenText(change.at, now, t, i18n.language)}
      </span>
    </button>
  )
}
