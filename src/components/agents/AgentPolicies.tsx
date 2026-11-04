/**
 * What the policies in `.nandi/policies/` do to one agent: which apply, what
 * each took away or narrowed from its skill.md, or why it can't run at all.
 * The rest of the Permissions section shows what skill.md asks for; this
 * is what it actually gets (`services::policies`).
 */
import { useTranslation } from 'react-i18next'
import { ShieldAlert, ShieldCheck } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import { useAgentPolicies } from '@/hooks/useAgentPolicies'
import type { ChangeOutcome, PolicyChange } from '@/lib/bindings'
import { cn } from '@/lib/utils'

const OUTCOME_CLASSES: Record<ChangeOutcome, string> = {
  removed: 'border-destructive/30 bg-destructive/10 text-destructive',
  narrowed: 'border-warning/30 bg-warning/10 text-warning',
  mustAsk: 'border-warning/30 bg-warning/10 text-warning',
  onHost: 'border-info/30 bg-info/10 text-info',
}

export function AgentPolicies({ agentId }: { agentId: string }) {
  const { t } = useTranslation()
  const policies = useAgentPolicies(agentId)

  if (policies.isPending) {
    return <Skeleton className="h-10 w-full" />
  }
  if (policies.isError) {
    return <p className="text-sm text-destructive">{policies.error.message}</p>
  }
  const { blocked, changes } = policies.data
  if (blocked) {
    return (
      <div
        role="alert"
        className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm"
      >
        <ShieldAlert
          aria-hidden
          className="mt-0.5 size-4 shrink-0 text-destructive"
        />
        <span>{t('agentPage.policies.blocked', { reason: blocked })}</span>
      </div>
    )
  }
  if (policies.data.policies.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {t('agentPage.policies.none')}
      </p>
    )
  }
  return (
    <div className="flex flex-col gap-2" data-testid="agent-policies">
      <p className="flex items-center gap-2 text-sm">
        <ShieldCheck aria-hidden className="size-4 shrink-0 text-success" />
        {t('agentPage.policies.applied', {
          list: policies.data.policies.join(', '),
        })}
      </p>
      {changes.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('agentPage.policies.unchanged')}
        </p>
      ) : (
        <ul
          aria-label={t('agentPage.policies.changes')}
          className="flex flex-col divide-y rounded-md border text-sm"
        >
          {changes.map(change => (
            <li
              key={`${change.item}:${change.by}:${change.outcome}`}
              className="flex flex-wrap items-center gap-2 px-3 py-2"
            >
              <span className="min-w-0 flex-1 font-mono text-xs" dir="ltr">
                {change.item}
              </span>
              <Badge
                variant="outline"
                className={cn('font-normal', OUTCOME_CLASSES[change.outcome])}
              >
                {outcomeLabel(t, change)}
              </Badge>
              <span className="text-xs text-muted-foreground">
                {t('agentPage.policies.by', { policy: change.by })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function outcomeLabel(
  t: ReturnType<typeof useTranslation>['t'],
  change: PolicyChange
): string {
  switch (change.outcome) {
    case 'removed':
      return t('agentPage.policies.outcome.removed')
    case 'narrowed':
      return t('agentPage.policies.outcome.narrowed', {
        detail: change.detail ?? '',
      })
    case 'mustAsk':
      return t('agentPage.policies.outcome.mustAsk')
    case 'onHost':
      return t('agentPage.policies.outcome.onHost')
  }
}
