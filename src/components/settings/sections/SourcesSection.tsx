import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import {
  Copy,
  ExternalLink,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Trash2,
} from 'lucide-react'
import { toast } from 'sonner'
import {
  AddMarketplaceSourceForm,
  CommunitySourceCard,
} from '@/components/marketplace/SourceForms'
import {
  isMarketplaceKind,
  MARKETPLACE_KIND_LABELS,
} from '@/components/marketplace/source-kinds'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { marketplaceQueryKeys, useRegistries } from '@/hooks/useMarketplace'
import type { RegistryInfo } from '@/lib/bindings'
import { client } from '@/lib/client'
import { marketplaceFilter } from '@/lib/marketplace-filter'
import { formatRelativeTime } from '@/lib/format-time'
import { removeRegistry } from '@/lib/marketplace-actions'
import { cn } from '@/lib/utils'
import { SettingsSectionFrame } from '../SettingsFrame'
import { isStale } from './sources-model'

/** Commands reject with the core's error string, not an `Error` */
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** Re-reads every agents & packs source (the core refetches them all) */
function useSyncMarketplace() {
  const queryClient = useQueryClient()
  const [syncing, setSyncing] = useState(false)
  const sync = async () => {
    setSyncing(true)
    const result = await client.listMarketplacePacks(
      marketplaceFilter({ refresh: true })
    )
    setSyncing(false)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    await queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  }
  return { sync, syncing }
}

/**
 * Settings › Sources: the registries the Library finds agents and packs
 * in. Each row has Sync and its last sync; Sync all re-reads them all.
 */
export function SourcesSection() {
  const { t } = useTranslation()
  const marketplace = useRegistries()
  const { sync, syncing } = useSyncMarketplace()
  const [adding, setAdding] = useState(false)
  const [removing, setRemoving] = useState<string | null>(null)

  const loading = marketplace.isPending
  const empty = !loading && (marketplace.data ?? []).length === 0

  return (
    <SettingsSectionFrame
      title={t('settings.section.sources')}
      description={t('settings.sources.description')}
      actions={
        <>
          <Button
            size="sm"
            variant="outline"
            disabled={syncing || loading}
            onClick={() => void sync()}
          >
            {syncing ? <Spinner /> : <RefreshCw />}
            {t('settings.sources.syncAll')}
          </Button>
          <Button size="sm" onClick={() => setAdding(value => !value)}>
            <Plus />
            {t('settings.sources.add')}
          </Button>
        </>
      }
    >
      {adding && <AddSource />}

      {marketplace.error && (
        <p role="alert" className="text-sm text-destructive">
          {errorText(marketplace.error)}
        </p>
      )}
      {loading ? (
        <Skeleton className="h-32" />
      ) : empty ? (
        <div className="flex items-center gap-3 rounded-lg border p-4 text-sm text-muted-foreground">
          <span className="flex-1">{t('settings.sources.empty')}</span>
          <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
            <Plus />
            {t('settings.sources.add')}
          </Button>
        </div>
      ) : (
        <ul className="divide-y rounded-lg border">
          {(marketplace.data ?? []).map(registry => (
            <MarketplaceRow
              key={registry.url}
              registry={registry}
              syncing={syncing}
              onSync={() => void sync()}
              onRemove={() => setRemoving(registry.url)}
            />
          ))}
        </ul>
      )}

      <AlertDialog
        open={removing !== null}
        onOpenChange={open => {
          if (!open) setRemoving(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('settings.sources.removeTitle', { url: removing })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.sources.removeDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (removing) void removeRegistry(removing)
              }}
            >
              {t('settings.sources.remove')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsSectionFrame>
  )
}

/** Add source: a registry's address, or a community registry of your own */
function AddSource() {
  return (
    <div className="grid gap-4 rounded-lg border p-4">
      <AddMarketplaceSourceForm />
      <CommunitySourceCard />
    </div>
  )
}

/** A row's frame: URL and labels, detail line, Sync, ⋯ and Remove */
function SourceRow({
  url,
  badges,
  detail,
  syncedAt,
  error,
  syncing,
  onSync,
  onRemove,
}: {
  url: string
  badges: React.ReactNode
  detail: React.ReactNode
  syncedAt: number | null
  error: string | null
  syncing: boolean
  onSync: () => void
  onRemove: () => void
}) {
  const { t, i18n } = useTranslation()
  const stale = isStale(syncedAt)
  const browsable = /^https?:\/\//i.test(url) || url.startsWith('github.com')

  return (
    <li
      className="flex flex-col gap-2 px-4 py-3"
      data-testid={`source-${url}`}
      data-settings-focus={url}
    >
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span dir="ltr" className="truncate font-mono text-sm">
              {url}
            </span>
            {badges}
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {detail}
            {' · '}
            <span className={cn(stale && 'text-warning')}>
              {syncedAt === null
                ? t('settings.sources.neverSynced')
                : t('settings.sources.synced', {
                    time: formatRelativeTime(syncedAt, i18n.language),
                  })}
            </span>
          </p>
        </div>
        <Button
          size="sm"
          variant="ghost"
          onClick={onSync}
          disabled={syncing}
          aria-label={t('settings.sources.syncLabel', { url })}
        >
          {syncing ? <Spinner /> : <RefreshCw />}
          {t('settings.sources.sync')}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="icon"
              variant="ghost"
              className="size-8"
              aria-label={t('settings.sources.more', { url })}
            >
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onSelect={() => void navigator.clipboard?.writeText(url)}
            >
              <Copy />
              {t('settings.sources.copyUrl')}
            </DropdownMenuItem>
            {browsable && (
              <DropdownMenuItem
                onSelect={() =>
                  void client.openExternal(
                    url.startsWith('http') ? url : `https://${url}`
                  )
                }
              >
                <ExternalLink />
                {t('settings.sources.openInBrowser')}
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          size="sm"
          variant="ghost"
          onClick={onRemove}
          aria-label={t('marketplace.removeRegistry', { url })}
        >
          <Trash2 />
          {t('settings.sources.remove')}
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </li>
  )
}

function MarketplaceRow({
  registry,
  syncing,
  onSync,
  onRemove,
}: {
  registry: RegistryInfo
  syncing: boolean
  onSync: () => void
  onRemove: () => void
}) {
  const { t } = useTranslation()
  return (
    <SourceRow
      url={registry.url}
      badges={
        <>
          <Badge variant="outline">
            {isMarketplaceKind(registry.kind)
              ? t(MARKETPLACE_KIND_LABELS[registry.kind])
              : registry.kind}
          </Badge>
          {registry.isDefault && (
            <Badge variant="outline">{t('marketplace.default')}</Badge>
          )}
          {registry.hasAuth && (
            <Badge variant="outline">{t('marketplace.private')}</Badge>
          )}
        </>
      }
      detail={
        registry.agentCount === null
          ? t('marketplace.registryUnread')
          : t('marketplace.registryCounts', {
              agents: registry.agentCount,
              packs: registry.packCount ?? 0,
            })
      }
      syncedAt={registry.fetchedAt ? Date.parse(registry.fetchedAt) : null}
      error={registry.warning}
      syncing={syncing}
      onSync={onSync}
      onRemove={onRemove}
    />
  )
}
