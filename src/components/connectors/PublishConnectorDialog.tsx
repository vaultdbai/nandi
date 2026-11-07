import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { usePublishConnector } from '@/hooks/useConnectorHub'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import { useRegistries } from '@/hooks/useMarketplace'
import { client } from '@/lib/client'
import type { PublishConnectorResult } from '@/lib/bindings'
import { publishTargets } from './scaffold-model'

/**
 * Publish a connector made here to one of the person's registries: a pull
 * request on a GitHub registry, or listed in place in a folder registry.
 */
export function PublishConnectorDialog({
  connector,
  onOpenChange,
}: {
  connector: ConnectorDetails | null
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={connector !== null} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        {connector && (
          <PublishBody
            key={connector.id}
            connector={connector}
            onDone={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function PublishBody({
  connector,
  onDone,
}: {
  connector: ConnectorDetails
  onDone: () => void
}) {
  const { t } = useTranslation()
  const registries = useRegistries()
  const targets = publishTargets(registries.data ?? [])
  const [picked, setPicked] = useState<string | null>(null)
  const registryUrl = picked ?? targets[0]?.url ?? ''
  const publish = usePublishConnector()
  const [result, setResult] = useState<PublishConnectorResult | null>(null)

  const submit = () =>
    publish.mutate(
      { connectorId: connector.id, registryUrl },
      { onSuccess: setResult }
    )

  return (
    <>
      <DialogHeader>
        <DialogTitle>
          {t('publishConnector.title', { name: connector.name })}
        </DialogTitle>
        <DialogDescription>
          {t('publishConnector.description')}
        </DialogDescription>
      </DialogHeader>
      {result ? (
        <div className="space-y-2 text-sm" data-testid="publish-result">
          {result.prUrl ? (
            <p>
              {t('publishConnector.prOpened')}{' '}
              <button
                type="button"
                className="text-primary underline-offset-2 hover:underline"
                dir="ltr"
                onClick={() => {
                  if (result.prUrl) void client.openExternal(result.prUrl)
                }}
              >
                {result.prUrl}
              </button>
            </p>
          ) : (
            <p>{t('publishConnector.listed', { registry: registryUrl })}</p>
          )}
          <p className="text-muted-foreground">
            {t('publishConnector.tagged', {
              tag: result.tag,
              version: result.version,
            })}
          </p>
          <p className="text-muted-foreground">
            {result.signed
              ? t('publishConnector.signed')
              : t('publishConnector.unsigned')}
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          {registries.isPending && <Spinner />}
          {registries.data && targets.length === 0 && (
            <p className="text-sm text-muted-foreground">
              {t('publishConnector.noRegistries')}
            </p>
          )}
          {targets.length > 0 && (
            <>
              <Label htmlFor="publish-registry">
                {t('publishConnector.registry')}
              </Label>
              <NativeSelect
                id="publish-registry"
                value={registryUrl}
                onChange={event => setPicked(event.target.value)}
              >
                {targets.map(registry => (
                  <NativeSelectOption key={registry.url} value={registry.url}>
                    {registry.url}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </>
          )}
          {publish.error && (
            <p className="text-sm text-destructive">{String(publish.error)}</p>
          )}
        </div>
      )}
      <DialogFooter>
        {result ? (
          <Button onClick={onDone}>{t('publishConnector.done')}</Button>
        ) : (
          <>
            <Button variant="outline" onClick={onDone}>
              {t('connectorConsent.cancel')}
            </Button>
            <Button
              disabled={publish.isPending || !registryUrl}
              onClick={submit}
            >
              {publish.isPending && <Spinner />}
              {t('packageConnectors.publish')}
            </Button>
          </>
        )}
      </DialogFooter>
    </>
  )
}
