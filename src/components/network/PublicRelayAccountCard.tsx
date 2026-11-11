/**
 * Network › Connection on the public relay: the email this Nandi joined
 * as (its connected mailbox), the account's Nandis, the free limits and
 * today's usage with when it resets, and a note that on a public relay
 * you see only the people you share a group with. While the relay doesn't
 * answer it names the account as kept, calmly.
 */
import { useTranslation } from 'react-i18next'
import { Users } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { usePublicRelayAccount } from '@/hooks/useRelay'
import { formatResetTime } from '@/lib/format-time'
import { useNavigationStore } from '@/store/navigation-store'

export function PublicRelayAccountCard() {
  const { t, i18n } = useTranslation()
  const account = usePublicRelayAccount(true)
  const view = account.data
  if (view?.kind === 'notPublic') return null

  return (
    <section
      aria-labelledby="public-account-title"
      className="flex flex-col gap-3 rounded-md border p-4"
      data-testid="public-relay-account"
    >
      <h2 id="public-account-title" className="text-sm font-semibold">
        {t('publicRelay.account.title')}
      </h2>
      {account.isPending ? (
        <Skeleton className="h-16 w-full" />
      ) : !view ? (
        <p className="text-sm text-muted-foreground">
          {t('publicRelay.account.unavailable')}
        </p>
      ) : view.kind === 'unavailable' ? (
        <>
          <p className="text-sm" data-testid="public-relay-joined-as">
            {t('publicRelay.account.joinedAs', { email: view.email })}
          </p>
          <p className="text-sm text-muted-foreground">
            {t('publicRelay.account.unavailable')}
          </p>
        </>
      ) : (
        <>
          <p className="text-sm" data-testid="public-relay-joined-as">
            {t('publicRelay.account.joinedAs', { email: view.account.email })}
          </p>
          <div className="grid gap-1">
            <h3 className="text-xs text-muted-foreground">
              {t('publicRelay.account.nandis', {
                count: view.account.instances.length,
              })}
            </h3>
            <ul
              className="grid gap-0.5 text-sm"
              data-testid="public-relay-nandis"
            >
              {view.account.instances.map(instance => (
                <li key={instance} className="flex items-center gap-2">
                  <code dir="ltr" className="font-mono text-xs break-all">
                    {instance}
                  </code>
                  {instance === view.thisInstance && (
                    <span className="text-xs text-muted-foreground">
                      {t('publicRelay.account.thisNandi')}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
          <dl
            className="grid gap-1 text-sm sm:grid-cols-3"
            data-testid="public-relay-usage"
          >
            <div>
              <dt className="text-xs text-muted-foreground">
                {t('publicRelay.account.groups')}
              </dt>
              <dd>
                {t('publicRelay.account.ofLimit', {
                  used: view.account.usage.groups,
                  limit: view.account.limits.groupsPerPerson,
                })}
              </dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">
                {t('publicRelay.account.membersPerGroup')}
              </dt>
              <dd>
                {t('publicRelay.account.upTo', {
                  limit: view.account.limits.membersPerGroup,
                })}
              </dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">
                {t('publicRelay.account.messagesToday')}
              </dt>
              <dd>
                {view.account.limits.messagesPerDay === null
                  ? t('publicRelay.account.noDailyLimit', {
                      used: view.account.usage.messagesToday,
                    })
                  : t('publicRelay.account.ofLimit', {
                      used: view.account.usage.messagesToday,
                      limit: view.account.limits.messagesPerDay,
                    })}
              </dd>
            </div>
          </dl>
          <p
            className="text-xs text-muted-foreground"
            data-testid="public-relay-resets"
          >
            {t('publicRelay.account.resets', {
              time: formatResetTime(view.account.usage.resetsAt, i18n.language),
            })}
          </p>
        </>
      )}
      <div className="flex flex-wrap items-center justify-between gap-2 border-t pt-3">
        <p className="text-sm text-muted-foreground">
          {t('publicRelay.account.groupsOnly')}
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'network', tab: 'groups' })
          }
        >
          <Users aria-hidden />
          {t('publicRelay.account.openGroups')}
        </Button>
      </div>
    </section>
  )
}
