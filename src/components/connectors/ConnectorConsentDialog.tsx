import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { VerificationBadge } from '@/components/marketplace/VerificationBadge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import {
  useConnectorConsent,
  useInstallRegistryConnector,
} from '@/hooks/useConnectorHub'
import type { ConnectorConsent, PackageTool } from '@/lib/bindings'
import {
  canInstall,
  entryState,
  runsWhere,
  signIn,
  toolsByAccess,
} from './consent-model'

export interface ConsentTarget {
  registryUrl: string
  connectorId: string
}

/**
 * Before a registry connector installs: what it reads and writes, the
 * hosts it reaches, how it signs in, where it runs, its tools and the
 * instructions agents get. Installing gives no agent access.
 */
export function ConnectorConsentDialog({
  target,
  onOpenChange,
}: {
  target: ConsentTarget | null
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const consent = useConnectorConsent(target)

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        {consent.isPending && target && <Spinner />}
        {consent.error && (
          <>
            <DialogHeader>
              <DialogTitle>{t('connectorConsent.loadFailed')}</DialogTitle>
              <DialogDescription>{String(consent.error)}</DialogDescription>
            </DialogHeader>
          </>
        )}
        {consent.data && target && (
          <ConsentBody
            key={`${target.registryUrl}:${target.connectorId}`}
            consent={consent.data}
            target={target}
            onDone={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function ConsentBody({
  consent,
  target,
  onDone,
}: {
  consent: ConnectorConsent
  target: ConsentTarget
  onDone: () => void
}) {
  const { t } = useTranslation()
  const [apiKey, setApiKey] = useState('')
  const install = useInstallRegistryConnector()
  const pkg = consent.package
  const where = runsWhere(pkg)
  const need = signIn(pkg.auth)
  const tools = toolsByAccess(pkg.tools)
  const state = entryState({
    version: pkg.version,
    installedVersion: consent.installedVersion,
  })

  const submit = () =>
    install.mutate(
      {
        registryUrl: target.registryUrl,
        connectorId: target.connectorId,
        apiKey: need.kind === 'apiKey' ? apiKey.trim() : null,
      },
      {
        onSuccess: () => {
          toast.success(t('connectorConsent.installed', { name: consent.name }))
          onDone()
        },
      }
    )

  return (
    <>
      <DialogHeader>
        <DialogTitle>
          {state === 'update'
            ? t('connectorConsent.updateTitle', { name: consent.name })
            : t('connectorConsent.title', { name: consent.name })}
        </DialogTitle>
        <DialogDescription>{consent.description}</DialogDescription>
      </DialogHeader>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>
          {t('connectorConsent.by', { publisher: consent.publisher })}
        </span>
        <VerificationBadge verified={consent.verified} />
        <Badge variant="outline">
          {state === 'update' && consent.installedVersion
            ? t('connectorConsent.versionChange', {
                from: consent.installedVersion,
                to: pkg.version,
              })
            : t('connectorConsent.version', { version: pkg.version })}
        </Badge>
      </div>

      <div className="space-y-4 text-sm">
        <ConsentList title={t('connectorConsent.reads')} items={pkg.reads} />
        <ConsentList title={t('connectorConsent.writes')} items={pkg.writes} />
        <ConsentList
          title={t('connectorConsent.hosts')}
          items={pkg.hosts}
          empty={t('connectorConsent.noHosts')}
          mono
        />
        <section className="space-y-1">
          <h3 className="font-medium">{t('connectorConsent.runsTitle')}</h3>
          <p className="text-muted-foreground">
            {where.key === 'image'
              ? t('connectorConsent.runs.image', { image: where.image })
              : where.key === 'server'
                ? t('connectorConsent.runs.server', { host: where.host })
                : t(`connectorConsent.runs.${where.key}`)}
          </p>
        </section>
        <section className="space-y-2">
          <h3 className="font-medium">{t('connectorConsent.signInTitle')}</h3>
          {need.kind === 'none' && (
            <p className="text-muted-foreground">
              {t('connectorConsent.signIn.none')}
            </p>
          )}
          {need.kind === 'unsupported' && (
            <p className="text-warning">
              {t('connectorConsent.signIn.unsupported')}
            </p>
          )}
          {need.kind === 'apiKey' && (
            <div className="space-y-1">
              <Label htmlFor="connector-api-key">
                {need.label ?? t('connectorConsent.signIn.apiKey')}
              </Label>
              <Input
                id="connector-api-key"
                type="password"
                dir="ltr"
                autoComplete="off"
                value={apiKey}
                onChange={event => setApiKey(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                {t('connectorConsent.signIn.keyring')}
              </p>
            </div>
          )}
        </section>
        {pkg.tools.length > 0 && (
          <section className="space-y-1">
            <h3 className="font-medium">{t('connectorConsent.tools')}</h3>
            <ToolList tools={tools.read} />
            <ToolList tools={tools.write} />
          </section>
        )}
        {consent.instructions.trim() && (
          <Collapsible>
            <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-2 hover:underline">
              {t('connectorConsent.instructions')}
            </CollapsibleTrigger>
            <CollapsibleContent>
              <p className="mt-2 rounded-md border bg-muted/60 px-3 py-2 text-xs whitespace-pre-wrap">
                {consent.instructions}
              </p>
            </CollapsibleContent>
          </Collapsible>
        )}
        <p className="rounded-md border px-3 py-2">
          {t('connectorConsent.noAccess')}
        </p>
        {install.error && (
          <p className="text-destructive">{String(install.error)}</p>
        )}
      </div>

      <DialogFooter>
        <Button variant="outline" onClick={onDone}>
          {t('connectorConsent.cancel')}
        </Button>
        <Button
          disabled={install.isPending || !canInstall(pkg.auth, apiKey)}
          onClick={submit}
        >
          {install.isPending && <Spinner />}
          {state === 'update'
            ? t('connectorConsent.update')
            : t('connectorConsent.install')}
        </Button>
      </DialogFooter>
    </>
  )
}

function ConsentList({
  title,
  items,
  empty,
  mono,
}: {
  title: string
  items: string[]
  empty?: string
  mono?: boolean
}) {
  if (items.length === 0 && !empty) return null
  return (
    <section className="space-y-1">
      <h3 className="font-medium">{title}</h3>
      {items.length === 0 ? (
        <p className="text-muted-foreground">{empty}</p>
      ) : (
        <ul className="list-disc space-y-0.5 ps-5 text-muted-foreground">
          {items.map(item => (
            <li key={item} dir={mono ? 'ltr' : undefined}>
              {mono ? <code className="text-xs">{item}</code> : item}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function ToolList({ tools }: { tools: PackageTool[] }) {
  const { t } = useTranslation()
  return (
    <ul className="space-y-1">
      {tools.map(tool => (
        <li key={tool.name} className="flex items-start gap-2">
          <Badge
            variant={tool.access === 'read' ? 'secondary' : 'outline'}
            className="shrink-0"
          >
            {t(`grants.access.${tool.access}`)}
          </Badge>
          <span>
            <code className="text-xs" dir="ltr">
              {tool.name}
            </code>
            {tool.description && (
              <span className="text-muted-foreground"> {tool.description}</span>
            )}
          </span>
        </li>
      ))}
    </ul>
  )
}
