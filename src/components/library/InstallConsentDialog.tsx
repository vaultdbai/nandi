import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ShieldAlert } from 'lucide-react'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { capabilityNotice } from '@/lib/marketplace-capabilities'
import type { ManifestCapabilities } from '@/lib/bindings'
import { PackConnectorsNotice } from '@/components/connectors/PackConnectorsNotice'
import { installAndAllow } from './library-actions'
import type { LibraryEntry } from './library-entry'

/**
 * One dialog, every time, before anything downloads (§6.1): what the item
 * will be allowed to do, a warning when nothing proves who published it,
 * then Install and allow (or Install anyway). Cancel downloads nothing.
 * `localPath`: the pack comes from a folder or `.zip` on this computer.
 */
export function InstallConsentDialog({
  entry,
  onOpenChange,
  localPath,
}: {
  entry: LibraryEntry | null
  onOpenChange: (open: boolean) => void
  localPath?: string
}) {
  const { t } = useTranslation()
  const [installing, setInstalling] = useState(false)
  const source = entry?.source
  const target =
    source?.type === 'agent'
      ? {
          kind: 'agent' as const,
          capabilities: source.agent.capabilities,
          agentIds: [source.agent.id],
          agents: [] as string[],
          workflows: 0,
        }
      : source?.type === 'pack'
        ? {
            kind: 'pack' as const,
            capabilities: source.pack.capabilities,
            agentIds: source.pack.agents.map(agent => agent.id),
            agents: source.pack.agents.map(agent => agent.name),
            workflows: source.pack.workflows.length,
          }
        : null

  const install = async () => {
    if (!entry || !target) return
    setInstalling(true)
    const done = await installAndAllow({
      kind: target.kind,
      id: entry.item.id,
      name: entry.item.name,
      agentIds: target.agentIds,
      localPath,
      shown: target.capabilities,
    })
    setInstalling(false)
    if (done) onOpenChange(false)
  }

  return (
    <AlertDialog open={target !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        {entry && target && (
          <>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t('library.install.title', { name: entry.item.name })}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {[
                  localPath !== undefined && t('library.local.from'),
                  entry.item.publisher && `@${entry.item.publisher}`,
                  entry.item.verified
                    ? t('library.install.verified')
                    : t('library.install.unverified'),
                  entry.item.version && `v${entry.item.version}`,
                ]
                  .filter(Boolean)
                  .join(' · ')}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AccessList capabilities={target.capabilities} />
            {source?.type === 'pack' && (
              <PackConnectorsNotice
                required={source.pack.requiresConnectors}
                actions={false}
              />
            )}
            {target.kind === 'pack' && (
              <details className="text-sm">
                <summary className="cursor-pointer text-muted-foreground">
                  {t('library.install.packContents', {
                    agents: target.agents.length,
                    workflows: target.workflows,
                    count: target.workflows,
                  })}
                </summary>
                <ul className="mt-1 list-disc ps-5">
                  {target.agents.map(name => (
                    <li key={name}>{name}</li>
                  ))}
                </ul>
              </details>
            )}
            {!entry.item.verified && (
              <p
                role="alert"
                className="flex items-start gap-2 rounded-md border border-warning/30 bg-warning/5 p-2 text-sm"
              >
                <ShieldAlert
                  aria-hidden
                  className="mt-0.5 size-4 shrink-0 text-warning"
                />
                {t('library.install.unverifiedWarning')}
              </p>
            )}
            <AlertDialogFooter>
              <AlertDialogCancel disabled={installing}>
                {t('library.cancel')}
              </AlertDialogCancel>
              <Button
                variant={entry.item.verified ? 'default' : 'destructive'}
                disabled={installing}
                onClick={() => void install()}
              >
                {installing && <Spinner />}
                {entry.item.verified
                  ? t('library.install.allow')
                  : t('library.install.anyway')}
              </Button>
            </AlertDialogFooter>
          </>
        )}
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** "It will be allowed to: …", then what it can't do, in one line */
export function AccessList({
  capabilities,
}: {
  capabilities: ManifestCapabilities
}) {
  const { t } = useTranslation()
  const { can } = capabilityNotice(capabilities)
  const cannot = [
    capabilities.pilots.length === 0 && t('library.access.noPilots'),
    capabilities.aws.length === 0 && t('library.access.noAws'),
  ].filter(Boolean)
  return (
    <div className="space-y-1 text-sm" data-testid="access-list">
      {can.length === 0 ? (
        <p>{t('library.access.nothing')}</p>
      ) : (
        <>
          <p className="font-medium">{t('library.access.allowedTo')}</p>
          <ul className="list-disc space-y-0.5 ps-5">
            {can.map(notice => (
              <li key={`${notice.key}-${JSON.stringify(notice.values ?? {})}`}>
                {t(notice.key, notice.values)}
              </li>
            ))}
          </ul>
        </>
      )}
      {cannot.length > 0 && (
        <p className="text-muted-foreground">{cannot.join(' ')}</p>
      )}
    </div>
  )
}
