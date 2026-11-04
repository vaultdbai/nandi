import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { GitFork, ScrollText, ShieldAlert, Store } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useAgentConsent } from '@/hooks/useMarketplace'
import type { AgentInfo } from '@/lib/bindings'
import { ConsentDialog } from './ConsentDialog'
import { ForkDialog } from './ForkDialog'

/**
 * On a marketplace agent: the user changes its rules in place and forks it
 * to change anything else, and it won't run until they approve what it
 * requests.
 */
export function SharedAgentBanner({
  agent,
  onOpenRules,
}: {
  agent: AgentInfo
  onOpenRules: () => void
}) {
  const { t } = useTranslation()
  const shared = agent.origin.kind !== 'local'
  const consent = useAgentConsent(agent.id, shared)
  const [forking, setForking] = useState(false)
  const [reviewing, setReviewing] = useState(false)

  if (!shared) return null
  const source =
    agent.origin.kind === 'pack'
      ? t('marketplace.banner.fromPack', { pack: agent.origin.pack })
      : agent.origin.kind === 'publisher'
        ? t('marketplace.banner.fromPublisher', {
            publisher: agent.origin.publisher,
          })
        : ''
  const needsConsent = consent.data !== undefined && !consent.data.satisfied

  return (
    <div className="flex flex-col gap-2 border-b bg-muted/40 px-4 py-2 text-sm">
      <div className="flex items-center gap-2">
        <Store className="size-4 shrink-0 text-muted-foreground" />
        <p className="min-w-0 flex-1">
          {t('marketplace.banner.shared')}{' '}
          <span className="text-muted-foreground">{source}</span>
        </p>
        <Button size="sm" variant="outline" onClick={onOpenRules}>
          <ScrollText />
          {t('marketplace.banner.rules')}
        </Button>
        <Button size="sm" variant="outline" onClick={() => setForking(true)}>
          <GitFork />
          {t('marketplace.fork')}
        </Button>
      </div>
      {needsConsent && (
        <div className="flex items-center gap-2" role="status">
          <ShieldAlert className="size-4 shrink-0 text-warning" />
          <p className="min-w-0 flex-1">{t('marketplace.banner.consent')}</p>
          <Button size="sm" onClick={() => setReviewing(true)}>
            {t('marketplace.banner.review')}
          </Button>
        </div>
      )}
      <ForkDialog
        agent={forking ? { id: agent.id, name: agent.name } : null}
        onOpenChange={setForking}
      />
      <ConsentDialog
        status={reviewing && consent.data ? consent.data : null}
        onOpenChange={setReviewing}
      />
    </div>
  )
}
