import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { UserRoundPlus } from 'lucide-react'
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
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { rawError } from '@/lib/friendly-error'
import {
  requesterName,
  useRespondTrustRequest,
  useTrustRequests,
} from '@/hooks/useTrustRequests'
import type { TrustRequest } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

/**
 * People refused because you don't trust them yet, one row each, with
 * Trust and Deny ("and block" asks first). Hidden when there are none.
 * No "Trust once": a refused request isn't held, so nothing could resume.
 */
export function TrustRequestsStrip() {
  const { t } = useTranslation()
  const requests = useTrustRequests()
  const list = requests.data ?? []
  if (list.length === 0) return null

  return (
    <section
      aria-labelledby="trust-requests-title"
      className="flex flex-col gap-2 rounded-md border border-warning/50 bg-warning/5 p-3"
      data-testid="trust-requests"
    >
      <h2
        id="trust-requests-title"
        className="flex items-center gap-2 text-sm font-semibold"
      >
        <UserRoundPlus aria-hidden className="size-4" />
        {t('network.trustRequests.title', { count: list.length })}
      </h2>
      <ul className="flex flex-col divide-y">
        {list.map(request => (
          <TrustRequestItem key={request.instanceId} request={request} />
        ))}
      </ul>
    </section>
  )
}

function TrustRequestItem({ request }: { request: TrustRequest }) {
  const { t, i18n } = useTranslation()
  const respond = useRespondTrustRequest()
  const [block, setBlock] = useState(false)
  const [asking, setAsking] = useState(false)
  const name = requesterName(request, t('network.unnamed'))
  const checkboxId = `block-${request.instanceId}`

  const answer = (trust: boolean) =>
    respond.mutate(
      {
        instanceId: request.instanceId,
        answer: trust ? 'trust' : 'deny',
        block: !trust && block,
      },
      {
        onSuccess: () =>
          toast.success(
            t(
              trust
                ? 'network.trustRequests.trusted'
                : block
                  ? 'network.trustRequests.blocked'
                  : 'network.trustRequests.denied',
              { name }
            )
          ),
        onError: error => toast.error(rawError(error)),
      }
    )

  return (
    <li
      className="flex flex-wrap items-center justify-between gap-3 py-2"
      data-testid={`trust-request-${request.instanceId}`}
    >
      <div className="min-w-0 space-y-0.5">
        <p className="text-sm">
          {t('network.trustRequests.wants', {
            name,
            count: request.attempts,
            when: formatRelativeTime(request.lastAt, i18n.language),
          })}
        </p>
        <p className="text-xs text-muted-foreground">
          {t('network.trustRequests.what', {
            capability: request.capability,
            agent: request.agentName,
          })}
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1.5">
          <Checkbox
            id={checkboxId}
            checked={block}
            onCheckedChange={checked => setBlock(checked === true)}
          />
          <Label htmlFor={checkboxId} className="text-xs font-normal">
            {t('network.trustRequests.andBlock')}
          </Label>
        </div>
        <Button
          size="sm"
          variant="outline"
          disabled={respond.isPending}
          onClick={() => (block ? setAsking(true) : answer(false))}
        >
          {t('network.trustRequests.deny')}
        </Button>
        <Button
          size="sm"
          disabled={respond.isPending}
          onClick={() => answer(true)}
        >
          {t('network.trustRequests.trust')}
        </Button>
      </div>
      <AlertDialog open={asking} onOpenChange={setAsking}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('network.trustRequests.blockTitle', { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('network.trustRequests.blockBody', { name })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('network.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => answer(false)}>
              {t('network.trustRequests.blockConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </li>
  )
}
