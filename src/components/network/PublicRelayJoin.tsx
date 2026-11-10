/**
 * "Join the public relay", shared by onboarding and Network › Connection:
 * asks whether the public relay can be joined now, offers its sign-in
 * providers, then shows the code to enter and "Open sign-in page" while
 * it polls every `interval` seconds, and says when this Nandi is linked
 * and connecting. An unreachable relay is a calm line, never a blocker.
 * The token never reaches the UI.
 */
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, ExternalLink } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { client } from '@/lib/client'
import type { RelaySignInProvider } from '@/lib/bindings'
import { relayQueryKeys, usePublicRelayOffer } from '@/hooks/useRelay'
import {
  afterPoll,
  afterStart,
  CHOOSE,
  offeredProviders,
  providerKey,
  type SignInState,
} from './public-relay-model'

/** The clock the sign-in's expiry is read against */
const now = () => Date.now()

export function PublicRelayJoin({
  onLinked,
}: {
  /** Called once the relay linked this Nandi (`relay`: its URL) */
  onLinked?: (relay: string) => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const offer = usePublicRelayOffer()
  const [state, setState] = useState<SignInState>(CHOOSE)

  // Asks again every `interval` seconds while waiting
  useEffect(() => {
    if (state.phase !== 'waiting') return
    const timer = setTimeout(() => {
      void (async () => {
        const result = await client.pollPublicRelaySignIn(
          state.provider,
          state.signinId
        )
        const next =
          result.status === 'error'
            ? ({
                phase: 'failed',
                failure: { key: 'publicRelay.failed', detail: result.error },
              } satisfies SignInState)
            : afterPoll(state, result.data, now())
        setState(next)
        if (next.phase === 'linked') {
          await queryClient.invalidateQueries({ queryKey: relayQueryKeys.all })
          onLinked?.(next.relay)
        }
      })()
    }, state.interval * 1000)
    return () => clearTimeout(timer)
  }, [state, queryClient, onLinked])

  const start = async (provider: RelaySignInProvider) => {
    setState({ phase: 'starting', provider })
    const result = await client.startPublicRelaySignIn(provider)
    setState(
      result.status === 'error'
        ? {
            phase: 'failed',
            failure: { key: 'publicRelay.failed', detail: result.error },
          }
        : afterStart(provider, result.data, now())
    )
  }

  if (state.phase === 'linked')
    return (
      <p
        role="status"
        data-testid="public-relay-linked"
        className="flex items-center gap-1 text-sm text-success"
      >
        <CheckCircle2 aria-hidden className="size-4 shrink-0" />
        {t('publicRelay.linked', {
          name: state.display,
          provider: t(providerKey(state.provider)),
        })}
      </p>
    )

  if (offer.isPending)
    return (
      <p
        role="status"
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <Spinner />
        {t('publicRelay.checking')}
      </p>
    )

  const data = offer.data
  if (offer.isError || !data || data.kind !== 'ready')
    return (
      <div className="space-y-2" data-testid="public-relay-unreachable">
        <p className="text-sm text-muted-foreground">
          {data?.kind === 'versionMismatch'
            ? t('publicRelay.versionMismatch')
            : t('publicRelay.unreachable')}
        </p>
        <Button
          size="sm"
          variant="outline"
          disabled={offer.isFetching}
          onClick={() => void offer.refetch()}
        >
          {offer.isFetching && <Spinner />}
          {t('publicRelay.tryAgain')}
        </Button>
      </div>
    )

  if (state.phase === 'waiting')
    return (
      <div
        className="flex flex-col gap-2 rounded-md border p-3 text-sm"
        data-testid="public-relay-waiting"
      >
        <p>
          {t('publicRelay.enterCode', {
            provider: t(providerKey(state.provider)),
          })}
        </p>
        <code
          dir="ltr"
          className="self-start rounded bg-muted px-2 py-1 font-mono text-lg tracking-wider select-all"
          data-testid="public-relay-user-code"
        >
          {state.userCode}
        </code>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            onClick={() => void client.openExternal(state.page)}
          >
            <ExternalLink aria-hidden />
            {t('publicRelay.openPage')}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setState(CHOOSE)}>
            {t('publicRelay.cancel')}
          </Button>
        </div>
        <p
          role="status"
          className="flex items-center gap-2 text-xs text-muted-foreground"
        >
          <Spinner />
          {state.retrying
            ? t('publicRelay.retrying')
            : t('publicRelay.waiting')}
        </p>
      </div>
    )

  const providers = offeredProviders(data.providers)
  return (
    <div className="flex flex-col gap-2" data-testid="public-relay-providers">
      <p className="text-xs text-muted-foreground">
        {t('publicRelay.limitsNote')}
      </p>
      <div className="flex flex-wrap gap-2">
        {providers.map(provider => (
          <Button
            key={provider}
            size="sm"
            variant="outline"
            disabled={state.phase === 'starting'}
            onClick={() => void start(provider)}
          >
            {state.phase === 'starting' && state.provider === provider && (
              <Spinner />
            )}
            {t('publicRelay.signInWith', {
              provider: t(providerKey(provider)),
            })}
          </Button>
        ))}
      </div>
      {state.phase === 'failed' && (
        <div
          role="alert"
          className="space-y-1"
          data-testid="public-relay-failed"
        >
          <p className="text-sm text-destructive">{t(state.failure.key)}</p>
          {state.failure.detail && (
            <p dir="auto" className="text-xs break-words text-muted-foreground">
              {state.failure.detail}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
