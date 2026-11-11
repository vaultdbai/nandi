/**
 * "Join the public relay", shared by onboarding and Network › Connection:
 * asks whether the public relay can be joined now and joins with this
 * Nandi's connected email in one click. Without a connected email it says
 * to connect Outlook or Gmail in Settings › Email (with the way there).
 * An unreachable relay is a calm line, never a blocker. The token never
 * reaches the UI.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, Mail } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { client } from '@/lib/client'
import { relayQueryKeys, usePublicRelayOffer } from '@/hooks/useRelay'
import { useNavigationStore } from '@/store/navigation-store'
import { afterJoin, IDLE, type JoinState } from './public-relay-model'

/** Settings › Email, where a mailbox is connected */
function openEmailSettings() {
  useNavigationStore.getState().navigate({ kind: 'settings', section: 'email' })
}

function NeedsEmail({ detail }: { detail?: string }) {
  const { t } = useTranslation()
  return (
    <div className="space-y-2" data-testid="public-relay-needs-email">
      <p className="text-sm text-muted-foreground">
        {t('publicRelay.needsEmail')}
      </p>
      {detail && (
        <p dir="auto" className="text-xs break-words text-muted-foreground">
          {detail}
        </p>
      )}
      <Button size="sm" variant="outline" onClick={openEmailSettings}>
        <Mail aria-hidden />
        {t('publicRelay.openEmail')}
      </Button>
    </div>
  )
}

export function PublicRelayJoin({
  onLinked,
}: {
  /** Called once this Nandi joined (`relay`: its URL) */
  onLinked?: (relay: string) => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const offer = usePublicRelayOffer()
  const [state, setState] = useState<JoinState>(IDLE)

  const join = async () => {
    setState({ phase: 'joining' })
    const result = await client.joinPublicRelay()
    const next: JoinState =
      result.status === 'error'
        ? {
            phase: 'failed',
            failure: { key: 'publicRelay.failed', detail: result.error },
          }
        : afterJoin(result.data)
    setState(next)
    if (next.phase === 'joined') {
      await queryClient.invalidateQueries({ queryKey: relayQueryKeys.all })
      onLinked?.(next.relay)
    }
  }

  if (state.phase === 'joined')
    return (
      <p
        role="status"
        data-testid="public-relay-linked"
        className="flex items-center gap-1 text-sm text-success"
      >
        <CheckCircle2 aria-hidden className="size-4 shrink-0" />
        {t('publicRelay.joined', { email: state.email })}
      </p>
    )

  if (state.phase === 'needsEmail') return <NeedsEmail detail={state.message} />

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
  if (data?.kind === 'needsEmail') return <NeedsEmail />

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

  return (
    <div className="flex flex-col gap-2" data-testid="public-relay-join">
      <p className="text-xs text-muted-foreground">
        {t('publicRelay.limitsNote')}
      </p>
      <p dir="auto" className="text-sm">
        {t('publicRelay.joinAs', { email: data.email })}
      </p>
      <div>
        <Button
          size="sm"
          disabled={state.phase === 'joining'}
          onClick={() => void join()}
        >
          {state.phase === 'joining' && <Spinner />}
          {state.phase === 'joining'
            ? t('publicRelay.joining')
            : t('publicRelay.join')}
        </Button>
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
