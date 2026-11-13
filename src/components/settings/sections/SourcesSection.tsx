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
import { AddCapabilitySourceForm } from '@/components/capability/AddCapabilitySourceForm'
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
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import {
  useCapabilityRegistries,
  useRemoveCapabilityRegistry,
  useSyncCapabilityRegistry,
} from '@/hooks/useCapabilityRegistries'
import { marketplaceQueryKeys, useRegistries } from '@/hooks/useMarketplace'
import type { CapabilityRegistry, RegistryInfo } from '@/lib/bindings'
import { client } from '@/lib/client'
import { marketplaceFilter } from '@/lib/marketplace-filter'
import { formatRelativeTime } from '@/lib/format-time'
import { removeRegistry } from '@/lib/marketplace-actions'
import { cn } from '@/lib/utils'
import { SettingsSectionFrame } from '../SettingsFrame'
import { isStale, type SourceKind } from './sources-model'

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
 * Settings › Sources: where the Library finds things, as one list with two
 * labelled kinds (Agents & packs, Capabilities). Each row has Sync and its
 * last sync; Sync all syncs them in turn.
 */
export function SourcesSection() {
  const { t } = useTranslation()
  const marketplace = useRegistries()
  const capabilities = useCapabilityRegistries()
  const syncCapability = useSyncCapabilityRegistry()
  const removeCapability = useRemoveCapabilityRegistry()
  const { sync: syncMarketplace, syncing: marketplaceSyncing } =
    useSyncMarketplace()
  const [syncingUrl, setSyncingUrl] = useState<string | null>(null)
  // Files the last sync of each capability source skipped
  const [warnings, setWarnings] = useState<Record<string, string[]>>({})
  const [syncingAll, setSyncingAll] = useState(false)
  const [adding, setAdding] = useState(false)
  const [removing, setRemoving] = useState<{
    kind: SourceKind
    url: string
  } | null>(null)

  const syncOne = async (registry: CapabilityRegistry) => {
    setSyncingUrl(registry.url)
    try {
      const result = await syncCapability.mutateAsync(registry.url)
      setWarnings(all => ({
        ...all,
        [registry.url]: result.error ? [] : result.warnings,
      }))
    } catch {
      // The row shows the registry's lastError
    } finally {
      setSyncingUrl(null)
    }
  }

  const syncAll = async () => {
    setSyncingAll(true)
    await syncMarketplace()
    for (const registry of capabilities.data ?? []) await syncOne(registry)
    setSyncingAll(false)
  }

  const loading = marketplace.isPending || capabilities.isPending
  const empty =
    !loading &&
    (marketplace.data ?? []).length === 0 &&
    (capabilities.data ?? []).length === 0

  return (
    <SettingsSectionFrame
      title={t('settings.section.sources')}
      description={t('settings.sources.description')}
      actions={
        <>
          <Button
            size="sm"
            variant="outline"
            disabled={syncingAll || loading}
            onClick={() => void syncAll()}
          >
            {syncingAll ? <Spinner /> : <RefreshCw />}
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

      {(marketplace.error ?? capabilities.error) && (
        <p role="alert" className="text-sm text-destructive">
          {errorText(marketplace.error ?? capabilities.error)}
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
              key={`agents:${registry.url}`}
              registry={registry}
              syncing={marketplaceSyncing}
              onSync={() => void syncMarketplace()}
              onRemove={() =>
                setRemoving({ kind: 'agents', url: registry.url })
              }
            />
          ))}
          {(capabilities.data ?? []).map(registry => (
            <CapabilityRow
              key={`capabilities:${registry.url}`}
              registry={registry}
              syncing={syncingUrl === registry.url}
              warnings={warnings[registry.url] ?? []}
              onSync={() => void syncOne(registry)}
              onRemove={() =>
                setRemoving({ kind: 'capabilities', url: registry.url })
              }
            />
          ))}
        </ul>
      )}
      {removeCapability.isError && (
        <p role="alert" className="text-sm text-destructive">
          {errorText(removeCapability.error)}
        </p>
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
              {t('settings.sources.removeTitle', { url: removing?.url })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.sources.removeDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (!removing) return
                if (removing.kind === 'agents')
                  void removeRegistry(removing.url)
                else removeCapability.mutate(removing.url)
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

/** Add source: the kind first, then that kind's form */
function AddSource() {
  const { t } = useTranslation()
  const [kind, setKind] = useState<SourceKind>('agents')
  return (
    <div className="flex flex-col gap-4 rounded-lg border p-4">
      <RadioGroup
        value={kind}
        onValueChange={value => setKind(value as SourceKind)}
        className="flex flex-wrap gap-4"
        aria-label={t('settings.sources.kindLabel')}
      >
        {(['agents', 'capabilities'] as const).map(option => (
          <div key={option} className="flex items-center gap-2">
            <RadioGroupItem value={option} id={`source-kind-${option}`} />
            <Label htmlFor={`source-kind-${option}`} className="font-normal">
              {t(`settings.sources.kind.${option}`)}
            </Label>
          </div>
        ))}
      </RadioGroup>
      {kind === 'agents' ? (
        <div className="grid gap-4">
          <AddMarketplaceSourceForm />
          <CommunitySourceCard />
        </div>
      ) : (
        <AddCapabilitySourceForm />
      )}
    </div>
  )
}

/** A row's frame: URL, kind and labels, detail line, Sync, ⋯ and Remove */
function SourceRow({
  url,
  kind,
  badges,
  detail,
  syncedAt,
  error,
  syncing,
  onSync,
  onRemove,
  children,
}: {
  url: string
  kind: SourceKind
  badges: React.ReactNode
  detail: React.ReactNode
  syncedAt: number | null
  error: string | null
  syncing: boolean
  onSync: () => void
  onRemove: () => void
  children?: React.ReactNode
}) {
  const { t, i18n } = useTranslation()
  const stale = isStale(syncedAt)
  const browsable = /^https?:\/\//i.test(url) || url.startsWith('github.com')

  return (
    <li
      className="flex flex-col gap-2 px-4 py-3"
      data-testid={`source-${kind}-${url}`}
      data-settings-focus={url}
    >
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span dir="ltr" className="truncate font-mono text-sm">
              {url}
            </span>
            <Badge variant="secondary">
              {t(`settings.sources.kind.${kind}`)}
            </Badge>
            {badges}
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {detail}
            {' · '}
            <span className={cn(stale && 'text-warning')}>
              {syncedAt === null
                ? t('capabilityRegistries.neverSynced')
                : t('capabilityRegistries.synced', {
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
          aria-label={t('capabilityRegistries.syncLabel', { url })}
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
      {children}
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
      kind="agents"
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

function CapabilityRow({
  registry,
  syncing,
  warnings,
  onSync,
  onRemove,
}: {
  registry: CapabilityRegistry
  syncing: boolean
  warnings: string[]
  onSync: () => void
  onRemove: () => void
}) {
  const { t } = useTranslation()
  return (
    <SourceRow
      url={registry.url}
      kind="capabilities"
      badges={
        <Badge variant="outline">
          {t(`capabilityRegistries.kind.${registry.kind}`)}
        </Badge>
      }
      detail={
        <>
          {t('capabilityRegistries.count', {
            count: registry.capabilityCount,
          })}
          {registry.lastCommit && (
            <>
              {' · '}
              <span dir="ltr" className="font-mono">
                {t('capabilityRegistries.commit', {
                  commit: registry.lastCommit.slice(0, 7),
                })}
              </span>
            </>
          )}
        </>
      }
      syncedAt={registry.lastSyncedAt}
      error={registry.lastError}
      syncing={syncing}
      onSync={onSync}
      onRemove={onRemove}
    >
      {warnings.length > 0 && (
        <div className="text-xs text-muted-foreground">
          <p>{t('capabilityRegistries.warnings')}</p>
          <ul className="list-disc ps-5">
            {warnings.map(warning => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      )}
    </SourceRow>
  )
}
