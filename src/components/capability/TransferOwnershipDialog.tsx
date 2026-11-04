import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { AlertTriangle } from 'lucide-react'
import { toast } from 'sonner'
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
import { Textarea } from '@/components/ui/textarea'
import {
  useCapability,
  useTransferCapabilityOwnership,
} from '@/hooks/useCapabilities'
import { errorMessage } from '@/hooks/useDelegation'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import type { CapabilityOwner, NetworkMember } from '@/lib/bindings'

/** The owner a network member becomes: their handle, else their instance */
function ownerFromMember(member: NetworkMember): CapabilityOwner {
  return {
    kind: 'user',
    id: member.handle ?? member.instanceId,
    displayName: member.displayName,
    transferable: true,
  }
}

/**
 * Pick a new owner from the network members, give a reason, confirm the
 * consequences; calls `transferCapabilityOwnership`.
 */
export function TransferOwnershipDialog({
  capabilityId,
  open,
  onOpenChange,
}: {
  capabilityId: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const capability = useCapability(open ? capabilityId : null)
  const network = useNetworkSnapshot(open)
  const transfer = useTransferCapabilityOwnership()
  const [memberId, setMemberId] = useState('')
  const [reason, setReason] = useState('')
  const [confirming, setConfirming] = useState(false)

  const currentOwner = capability.data?.owner.id
  const candidates = (network.data?.members ?? []).filter(
    member => ownerFromMember(member).id !== currentOwner
  )
  const chosen = candidates.find(member => member.instanceId === memberId)

  const close = (next: boolean) => {
    if (!next) {
      setMemberId('')
      setReason('')
      setConfirming(false)
    }
    onOpenChange(next)
  }

  const submit = async () => {
    if (!chosen) return
    try {
      await transfer.mutateAsync({
        id: capabilityId,
        newOwner: ownerFromMember(chosen),
        reason: reason.trim() === '' ? null : reason.trim(),
      })
      toast.success(
        t('capabilities.transfer.done', { name: chosen.displayName })
      )
      close(false)
    } catch (error) {
      toast.error(
        t('capabilities.transfer.failed', { error: errorMessage(error) })
      )
    }
  }

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('capabilities.transfer.title')}</DialogTitle>
          <DialogDescription>
            {t('capabilities.transfer.description', {
              name: capability.data?.name || capabilityId,
            })}
          </DialogDescription>
        </DialogHeader>
        {confirming && chosen ? (
          <div
            className="flex gap-3 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm"
            role="alert"
          >
            <AlertTriangle
              className="mt-0.5 size-4 shrink-0 text-destructive"
              aria-hidden
            />
            <div className="space-y-1">
              <p className="font-medium">
                {t('capabilities.transfer.confirmTitle', {
                  name: chosen.displayName,
                })}
              </p>
              <p className="text-muted-foreground">
                {t('capabilities.transfer.consequences')}
              </p>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
              <Label htmlFor="transfer-owner">
                {t('capabilities.transfer.newOwner')}
              </Label>
              <NativeSelect
                id="transfer-owner"
                value={memberId}
                onChange={event => setMemberId(event.target.value)}
                disabled={network.isPending}
              >
                <NativeSelectOption value="">
                  {network.isPending
                    ? t('capabilities.transfer.loadingMembers')
                    : t('capabilities.transfer.pickMember')}
                </NativeSelectOption>
                {candidates.map(member => (
                  <NativeSelectOption
                    key={member.instanceId}
                    value={member.instanceId}
                  >
                    {member.handle
                      ? `${member.displayName} (@${member.handle})`
                      : member.displayName}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
              {network.data && candidates.length === 0 && (
                <p className="text-xs text-muted-foreground">
                  {t('capabilities.transfer.noMembers')}
                </p>
              )}
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="transfer-reason">
                {t('capabilities.transfer.reason')}
              </Label>
              <Textarea
                id="transfer-reason"
                rows={3}
                value={reason}
                placeholder={t('capabilities.transfer.reasonPlaceholder')}
                onChange={event => setReason(event.target.value)}
              />
            </div>
          </div>
        )}
        <DialogFooter>
          {confirming ? (
            <>
              <Button variant="outline" onClick={() => setConfirming(false)}>
                {t('capabilities.back')}
              </Button>
              <Button
                variant="destructive"
                disabled={transfer.isPending}
                onClick={() => void submit()}
              >
                {transfer.isPending && <Spinner />}
                {t('capabilities.transfer.confirm')}
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={() => close(false)}>
                {t('capabilities.cancel')}
              </Button>
              <Button disabled={!chosen} onClick={() => setConfirming(true)}>
                {t('capabilities.transfer.continue')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
