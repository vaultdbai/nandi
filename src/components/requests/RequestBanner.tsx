import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CircleAlert, Hourglass, PauseCircle, Timer } from 'lucide-react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { useRetryRequest } from '@/hooks/useRequests'
import { decideApproval } from '@/lib/agent-actions'
import { client } from '@/lib/client'
import type { Blocker, Request } from '@/lib/bindings'
import { formatDuration } from '@/lib/format-time'
import { openSettings } from '@/lib/settings-nav'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import {
  bannerActions,
  bannerSentenceKey,
  bannerTone,
  isGatewayHold,
  needsConfirmation,
  type BannerAction,
} from './request-model'

const TONES = {
  warning: 'border-warning/40 bg-warning/5',
  neutral: 'border-border bg-muted/40',
  destructive: 'border-destructive/40 bg-destructive/5',
} as const

/** The one banner: why it's stuck, who acts, and the button that fixes it */
export function RequestBanner({
  request,
  blocker,
  now,
  onShowDetails,
}: {
  request: Request
  blocker: Blocker
  now: number
  onShowDetails: () => void
}) {
  const { t } = useTranslation()
  const retry = useRetryRequest()
  const [confirming, setConfirming] = useState<BannerAction | null>(null)
  const [busy, setBusy] = useState(false)
  const { primary, secondary } = bannerActions(request, blocker)
  const names = {
    member: blocker.member ?? t('request.someone'),
    agent: blocker.agentName ?? t('request.yourAgent'),
  }

  const result = async (
    promise: Promise<{ status: 'ok' } | { status: 'error'; error: string }>,
    done?: string
  ) => {
    setBusy(true)
    const outcome = await promise
    setBusy(false)
    if (outcome.status === 'error') {
      toast.error(outcome.error)
      return
    }
    if (done) toast.success(done)
  }

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success(t('request.copied', { member: names.member }))
    } catch {
      toast.error(t('request.copyFailed'))
    }
  }

  const run = async (action: BannerAction) => {
    const instance = blocker.memberInstance
    switch (action) {
      case 'approve':
      case 'reject':
        if (isGatewayHold(request, blocker)) {
          await result(
            client.decideGatewayHold(blocker.taskId, action === 'approve'),
            t(
              action === 'approve'
                ? 'request.done.holdApproved'
                : 'request.done.holdDeclined',
              names
            )
          )
          return
        }
        if (blocker.agentId && blocker.approvalId) {
          setBusy(true)
          await decideApproval(
            blocker.agentId,
            blocker.approvalId,
            action === 'approve' ? 'approved' : 'rejected'
          )
          setBusy(false)
        }
        return
      case 'answer':
        useNavigationStore
          .getState()
          .navigate({ kind: 'delegation-thread', taskId: blocker.taskId })
        return
      case 'trust':
        if (instance) {
          await result(
            client.setTrustRelationship(instance, {
              addedAsTrustedPeer: true,
              delegationsPaused: null,
            }),
            t('request.done.trusted', { member: names.member })
          )
        }
        return
      case 'block':
        if (instance) {
          await result(
            client.pauseDelegationsFromMember(instance, true),
            t('request.done.blocked', { member: names.member })
          )
        }
        return
      case 'resume':
        if (instance) {
          await result(
            client.pauseDelegationsFromMember(instance, false),
            t('request.done.resumed', { member: names.member })
          )
        }
        return
      case 'turnOn': {
        const agentId = blocker.agentId
        if (!agentId) return
        const current = await client.getAgentDelegation(agentId)
        if (current.status === 'error') {
          toast.error(current.error)
          return
        }
        await result(
          client.setAgentDelegation(agentId, {
            ...current.data,
            acceptInbound: true,
          }),
          t('request.done.turnedOn', { agent: names.agent })
        )
        return
      }
      case 'sendElsewhere':
      case 'retry':
        retry.mutate(request.id, {
          onError: error =>
            toast.error(error instanceof Error ? error.message : String(error)),
        })
        return
      case 'cancel':
        await result(
          client.cancelTask(blocker.taskId, null),
          t('request.done.cancelled')
        )
        return
      case 'addModel':
        openSettings('models')
        return
      case 'addKey':
        useNavigationStore.getState().navigate({
          kind: 'settings',
          section: 'models',
          focus: 'runtimes',
        })
        return
      case 'details':
        onShowDetails()
        return
      case 'askTrust':
      case 'askTurnOn':
      case 'remind':
        await copy(t(`request.message.${action}`, names))
        return
    }
  }

  const click = (action: BannerAction) =>
    needsConfirmation(action) ? setConfirming(action) : void run(action)

  const waited = formatDuration(Math.max(0, now - blocker.since))
  const Icon = blocker.terminal
    ? CircleAlert
    : blocker.who === 'me'
      ? PauseCircle
      : blocker.who === 'them'
        ? Hourglass
        : Timer

  return (
    <section
      role="alert"
      aria-label={t('request.banner.label')}
      data-testid="request-banner"
      data-kind={blocker.kind}
      className={cn(
        'flex flex-col gap-3 rounded-lg border px-4 py-3 sm:flex-row sm:items-center',
        TONES[bannerTone(blocker)]
      )}
    >
      <div className="flex min-w-0 flex-1 items-start gap-3">
        <Icon aria-hidden className="mt-0.5 size-5 shrink-0" />
        <div className="min-w-0 space-y-0.5">
          <p className="text-sm font-medium">
            {t(bannerSentenceKey(request, blocker), names)}
          </p>
          <p className="text-xs text-muted-foreground">
            {blocker.terminal
              ? t('request.banner.ended')
              : t('request.banner.waiting', { duration: waited })}
            {blocker.queuePosition !== null &&
              ` · ${t('request.banner.position', { position: blocker.queuePosition })}`}
            {blocker.deadline !== null &&
              !blocker.terminal &&
              ` · ${t('request.banner.expiresIn', {
                duration: formatDuration(Math.max(0, blocker.deadline - now)),
              })}`}
            {request.moreWaiting > 0 &&
              ` · ${t('request.banner.moreWaiting', { count: request.moreWaiting })}`}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {secondary.map(action => (
          <Button
            key={action}
            size="sm"
            variant="outline"
            disabled={busy || retry.isPending}
            onClick={() => click(action)}
          >
            {t(`request.action.${action}`, names)}
          </Button>
        ))}
        {primary && (
          <Button
            size="sm"
            disabled={busy || retry.isPending}
            onClick={() => click(primary)}
          >
            {t(`request.action.${primary}`, names)}
          </Button>
        )}
      </div>

      <AlertDialog
        open={confirming !== null}
        onOpenChange={open => !open && setConfirming(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirming && t(`request.confirm.${confirming}.title`, names)}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirming &&
                t(`request.confirm.${confirming}.description`, names)}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('request.confirm.keep')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (confirming) void run(confirming)
                setConfirming(null)
              }}
            >
              {confirming && t(`request.action.${confirming}`, names)}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
