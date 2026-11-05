import { useTranslation } from 'react-i18next'
import { MarketplaceIcon } from '@/components/marketplace/MarketplaceIcon'
import { VerificationBadge } from '@/components/marketplace/VerificationBadge'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { LibraryEntry } from './library-entry'
import { displayName, permissionsLine } from './library-format'
import type { LibraryItem, StatusPill } from './library-model'
import {
  LibraryItemAction,
  type LibraryItemActionProps,
} from './LibraryItemAction'

/** How many Can do names a card lists before "+n" */
const CAN_DO_SHOWN = 2
/** Tags after the kind */
const TAGS_SHOWN = 2

/**
 * One card for every kind (§4.2): name and one status pill, description,
 * kind and tags, Can do, one permissions line, publisher and the action.
 * The whole card opens the drawer; the name is the keyboard target. No
 * IDs, ports, usage counts or star ratings.
 */
export function LibraryCard({
  entry,
  onOpenDrawer,
  onInstall,
}: { entry: LibraryEntry } & Omit<LibraryItemActionProps, 'entry'>) {
  const { t } = useTranslation()
  const { item } = entry
  const isTool = item.kind === 'tools'
  const name = displayName(item, t)

  return (
    <article
      data-testid={`library-card-${item.key}`}
      onClick={() => onOpenDrawer(item)}
      className={cn(
        'flex cursor-pointer flex-col gap-2 rounded-xl border bg-card p-4 text-card-foreground shadow-xs transition-colors hover:bg-accent/30'
      )}
    >
      <div className="flex items-start gap-3">
        <MarketplaceIcon
          url={null}
          kind={iconKind(item)}
          className="size-9 shrink-0"
        />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">
            <button
              type="button"
              className="text-start hover:underline focus-visible:underline"
              onClick={event => {
                event.stopPropagation()
                onOpenDrawer(item)
              }}
            >
              {name}
            </button>
          </h3>
          <p className="line-clamp-2 text-xs text-muted-foreground">
            {item.type === 'workflow'
              ? t('library.card.partOf', { pack: item.packName })
              : item.description}
          </p>
        </div>
        {item.pill && <StatusBadge pill={item.pill} />}
      </div>

      <p className="text-xs text-muted-foreground">
        {[
          kindLabel(item, t),
          ...(isTool ? [] : item.tags.slice(0, TAGS_SHOWN)),
          ...(entry.source.type === 'builtinTool' &&
          entry.source.entry.definition.requiresApproval
            ? [t('library.card.asksFirst')]
            : []),
        ].join(' · ')}
      </p>
      {!isTool && item.canDo.length > 0 && (
        <p className="truncate text-xs">
          {t('library.card.canDo', {
            names: canDoLine(item.canDo, t),
          })}
        </p>
      )}
      {item.permissions && (
        <p className="text-xs" data-testid="permissions-line">
          {permissionsLine(item.permissions, t)}
        </p>
      )}

      <div
        className="mt-auto flex items-center justify-between gap-2 border-t pt-2"
        onClick={event => event.stopPropagation()}
      >
        <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          {item.publisher && (
            <span className="truncate">@{item.publisher}</span>
          )}
          {!isTool && item.publisher && (
            <VerificationBadge verified={item.verified} />
          )}
          {item.version && <span>v{item.version}</span>}
          {!item.compatible && !isTool && (
            <span className="text-warning">{t('library.card.needsNewer')}</span>
          )}
        </div>
        <LibraryItemAction
          entry={entry}
          onInstall={onInstall}
          onOpenDrawer={onOpenDrawer}
        />
      </div>
    </article>
  )
}

type T = (key: string, options?: Record<string, unknown>) => string

function iconKind(item: LibraryItem): 'pack' | 'agent' | 'workflow' | 'tool' {
  if (item.kind === 'tools') return 'tool'
  if (item.kind === 'packs') return 'pack'
  return item.kind === 'workflows' ? 'workflow' : 'agent'
}

function kindLabel(item: LibraryItem, t: T): string {
  return t(`library.type.${item.type}`)
}

function canDoLine(names: string[], t: T): string {
  const shown = names.slice(0, CAN_DO_SHOWN).join(', ')
  const more = names.length - CAN_DO_SHOWN
  return more > 0
    ? `${shown} ${t('library.card.more', { count: more })}`
    : shown
}

const PILL_TONES: Record<StatusPill, string> = {
  installed: 'border-success/40 text-success',
  running: 'border-success/40 text-success',
  stopped: 'text-muted-foreground',
  needsSetup: 'border-warning/40 text-warning',
  problem: 'border-destructive/40 text-destructive',
  updateAvailable: 'border-info/40 text-info',
  off: 'text-muted-foreground',
  notForThisComputer: 'text-muted-foreground',
}

export function StatusBadge({ pill }: { pill: StatusPill }) {
  const { t } = useTranslation()
  return (
    <Badge
      variant="outline"
      className={cn('shrink-0', PILL_TONES[pill])}
      data-testid="status-pill"
    >
      {t(`library.pill.${pill}`)}
    </Badge>
  )
}
