import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plug } from 'lucide-react'
import { toast } from 'sonner'
import { VerificationBadge } from '@/components/marketplace/VerificationBadge'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import {
  useRegistryConnectors,
  useRemoveConnector,
} from '@/hooks/useConnectorHub'
import { useConnectors, type ConnectorDetails } from '@/hooks/useConnectors'
import type { ConnectorEntry } from '@/lib/bindings'
import {
  ConnectorConsentDialog,
  type ConsentTarget,
} from './ConnectorConsentDialog'
import { entryState, filterEntries } from './consent-model'
import { PublishConnectorDialog } from './PublishConnectorDialog'

/**
 * Library › Connectors: Browse lists the registries' connectors with
 * Install or Update (through the consent dialog); Installed lists the
 * package connectors with their version, source, Remove and (made here)
 * Publish.
 */
export function PackageConnectorsSection({
  tab,
  search,
}: {
  tab: 'installed' | 'browse'
  search: string
}) {
  const { t } = useTranslation()
  return (
    <section className="mt-6 space-y-3" aria-labelledby="package-connectors">
      <h2 id="package-connectors" className="text-sm font-semibold">
        {tab === 'browse'
          ? t('packageConnectors.browseTitle')
          : t('packageConnectors.installedTitle')}
      </h2>
      {tab === 'browse' ? (
        <RegistryList search={search} />
      ) : (
        <InstalledList search={search} />
      )}
    </section>
  )
}

function SectionGrid({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      {children}
    </div>
  )
}

function RegistryList({ search }: { search: string }) {
  const { t } = useTranslation()
  const entries = useRegistryConnectors()
  const [target, setTarget] = useState<ConsentTarget | null>(null)
  if (entries.isPending) return <Skeleton className="h-24 rounded-xl" />
  if (entries.error) {
    return <p className="text-sm text-destructive">{String(entries.error)}</p>
  }
  const shown = filterEntries(entries.data, search)
  return (
    <>
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('packageConnectors.noneListed')}
        </p>
      ) : (
        <SectionGrid>
          {shown.map(entry => (
            <RegistryCard
              key={`${entry.registry}:${entry.id}`}
              entry={entry}
              onInstall={() =>
                setTarget({
                  registryUrl: entry.registry,
                  connectorId: entry.id,
                })
              }
            />
          ))}
        </SectionGrid>
      )}
      <ConnectorConsentDialog
        target={target}
        onOpenChange={open => !open && setTarget(null)}
      />
    </>
  )
}

function RegistryCard({
  entry,
  onInstall,
}: {
  entry: ConnectorEntry
  onInstall: () => void
}) {
  const { t } = useTranslation()
  const state = entryState(entry)
  return (
    <article
      data-testid={`registry-connector-${entry.id}`}
      className="flex flex-col gap-2 rounded-xl border bg-card p-4 text-card-foreground shadow-xs"
    >
      <div className="flex items-start gap-2">
        <Plug aria-hidden className="mt-0.5 size-4 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">{entry.name}</h3>
          <p className="text-xs text-muted-foreground">
            {t('connectorConsent.by', { publisher: entry.publisher })}
          </p>
        </div>
        {state !== 'install' && (
          <Badge variant="secondary">
            {state === 'update'
              ? t('packageConnectors.updateAvailable')
              : t('packageConnectors.installed')}
          </Badge>
        )}
      </div>
      <p className="line-clamp-2 text-sm text-muted-foreground">
        {entry.description}
      </p>
      <div className="mt-auto flex flex-wrap items-center gap-2">
        <VerificationBadge verified={entry.verified} />
        <Badge variant="outline">
          {t('connectorConsent.version', { version: entry.version })}
        </Badge>
        <span className="flex-1" />
        {state !== 'installed' && (
          <Button size="sm" variant="outline" onClick={onInstall}>
            {state === 'update'
              ? t('connectorConsent.update')
              : t('connectorConsent.install')}
          </Button>
        )}
      </div>
    </article>
  )
}

function InstalledList({ search }: { search: string }) {
  const { t } = useTranslation()
  const connectors = useConnectors()
  const [publishing, setPublishing] = useState<ConnectorDetails | null>(null)
  const query = search.trim().toLowerCase()
  const packages = (connectors.data ?? []).filter(
    connector =>
      connector.package &&
      (!query || connector.name.toLowerCase().includes(query))
  )
  if (connectors.isPending) return <Skeleton className="h-24 rounded-xl" />
  return (
    <>
      {packages.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('packageConnectors.noneInstalled')}
        </p>
      ) : (
        <SectionGrid>
          {packages.map(connector => (
            <InstalledCard
              key={connector.id}
              connector={connector}
              onPublish={() => setPublishing(connector)}
            />
          ))}
        </SectionGrid>
      )}
      <PublishConnectorDialog
        connector={publishing}
        onOpenChange={open => !open && setPublishing(null)}
      />
    </>
  )
}

function InstalledCard({
  connector,
  onPublish,
}: {
  connector: ConnectorDetails
  onPublish: () => void
}) {
  const { t } = useTranslation()
  const remove = useRemoveConnector()
  const pkg = connector.package
  if (!pkg) return null
  const madeHere = pkg.registry === null
  return (
    <article
      data-testid={`package-connector-${connector.id}`}
      className="flex flex-col gap-2 rounded-xl border bg-card p-4 text-card-foreground shadow-xs"
    >
      <div className="flex items-start gap-2">
        <Plug aria-hidden className="mt-0.5 size-4 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-medium">{connector.name}</h3>
          <p className="truncate text-xs text-muted-foreground" dir="auto">
            {madeHere
              ? t('packageConnectors.madeHere')
              : t('packageConnectors.from', {
                  source: pkg.publisher ?? pkg.registry,
                })}
          </p>
        </div>
        <Badge variant="outline">
          {t('connectorConsent.version', { version: pkg.version })}
        </Badge>
      </div>
      <p className="line-clamp-2 text-sm text-muted-foreground">
        {connector.description}
      </p>
      <div className="mt-auto flex flex-wrap justify-end gap-2">
        {madeHere && (
          <Button size="sm" variant="outline" onClick={onPublish}>
            {t('packageConnectors.publish')}
          </Button>
        )}
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button size="sm" variant="ghost" disabled={remove.isPending}>
              {t('packageConnectors.remove')}
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t('packageConnectors.removeTitle', { name: connector.name })}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {t('packageConnectors.removeBody')}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t('grants.keep')}</AlertDialogCancel>
              <AlertDialogAction
                onClick={() =>
                  remove.mutate(connector.id, {
                    onError: error =>
                      toast.error(t('packageConnectors.removeFailed'), {
                        description: String(error),
                      }),
                  })
                }
              >
                {t('packageConnectors.remove')}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </article>
  )
}
