import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plug } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useRegistryConnectors } from '@/hooks/useConnectorHub'
import { useConnectors } from '@/hooks/useConnectors'
import {
  ConnectorConsentDialog,
  type ConsentTarget,
} from './ConnectorConsentDialog'
import { missingConnectors } from './consent-model'

/**
 * A pack that needs connectors not installed here: "This pack uses: …",
 * each with Install (through the consent dialog) when a registry lists it.
 */
export function PackConnectorsNotice({
  required,
  actions = true,
}: {
  required: string[]
  /** Install buttons; off inside another dialog */
  actions?: boolean
}) {
  const { t } = useTranslation()
  const connectors = useConnectors()
  const registry = useRegistryConnectors()
  const [target, setTarget] = useState<ConsentTarget | null>(null)
  if (required.length === 0 || !connectors.data) return null
  const installed = connectors.data
    .filter(c => c.status.installed)
    .map(c => c.id)
  const missing = missingConnectors(required, installed, registry.data ?? [])
  if (missing.length === 0) return null

  return (
    <div
      className="space-y-2 rounded-md border border-warning/30 bg-warning/5 p-3 text-sm"
      data-testid="pack-connectors"
    >
      <p className="flex items-center gap-2 font-medium">
        <Plug aria-hidden className="size-4 text-warning" />
        {t('packConnectors.uses', {
          names: missing.map(m => m.entry?.name ?? m.id).join(', '),
        })}
      </p>
      {actions && (
        <ul className="space-y-1">
          {missing.map(({ id, entry }) => (
            <li key={id} className="flex items-center gap-2">
              <span className="flex-1">{entry?.name ?? id}</span>
              {entry ? (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    setTarget({ registryUrl: entry.registry, connectorId: id })
                  }
                >
                  {t('connectorConsent.install')}
                </Button>
              ) : (
                <span className="text-xs text-muted-foreground">
                  {t('packConnectors.notListed')}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {actions && (
        <ConnectorConsentDialog
          target={target}
          onOpenChange={open => !open && setTarget(null)}
        />
      )}
    </div>
  )
}
