import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import type { ManifestCapabilities } from '@/lib/bindings'

/** Network hosts shown before collapsing into "+N" */
const MAX_HOSTS = 2

/**
 * What an agent (or a pack's agents together) will be able to use, at a
 * glance: each tool, the connectors it uses, the network hosts, and pilots.
 */
export function CapabilityBadges({
  capabilities,
}: {
  capabilities: ManifestCapabilities
}) {
  const { t } = useTranslation()
  const { tools, network, pilots, connectors } = capabilities
  const hosts = network.slice(0, MAX_HOSTS)
  const moreHosts = network.length - hosts.length

  return (
    <ul
      className="flex flex-wrap gap-1"
      aria-label={t('marketplace.capabilities')}
    >
      {tools.map(tool => (
        <li key={`tool-${tool}`}>
          <Badge variant="secondary" className="font-mono">
            {tool}
          </Badge>
        </li>
      ))}
      {connectors.map(connector => (
        <li key={`connector-${connector}`}>
          <Badge variant="secondary">
            {t('marketplace.connector', { connector })}
          </Badge>
        </li>
      ))}
      {network.length === 0 ? (
        <li>
          <Badge variant="outline">{t('marketplace.noNetwork')}</Badge>
        </li>
      ) : (
        hosts.map(host => (
          <li key={`host-${host}`}>
            <Badge variant="outline">
              {t('marketplace.networkHost', { host })}
            </Badge>
          </li>
        ))
      )}
      {moreHosts > 0 && (
        <li>
          <Badge variant="outline">
            {t('marketplace.moreHosts', { count: moreHosts })}
          </Badge>
        </li>
      )}
      <li>
        <Badge variant="outline">
          {pilots.length === 0
            ? t('marketplace.noPilots')
            : t('marketplace.pilots', { pilots: pilots.join(', ') })}
        </Badge>
      </li>
    </ul>
  )
}
