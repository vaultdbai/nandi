import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  useApproveQueuedWork,
  useDismissQueuedWork,
} from '@/hooks/useChiefOfStaffDigest'
import { useViewOpener } from '@/hooks/useViewOpener'
import { rowView, type QueueRow } from '@/components/home/digest-model'

/**
 * Approve and Dismiss for queued intake items, with the confirm step for
 * suspicious ones and the toasts. Shared by Home's card and the thread page.
 * `opens` is where the toast's "Open" goes: the item's thread (Home), or
 * the Request it started (the thread page is already open).
 */
export function useQueueDecisions(opens: 'thread' | 'request') {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const approve = useApproveQueuedWork()
  const dismiss = useDismissQueuedWork()
  const [confirming, setConfirming] = useState<QueueRow | null>(null)

  const approveRow = async (row: QueueRow) => {
    setConfirming(null)
    try {
      const requestId = await approve.mutateAsync(row.id)
      const message =
        row.action === 'update' && row.requestTitle
          ? t('home.chiefOfStaffDigest.queue.toast.addedTo', {
              title: row.requestTitle,
            })
          : t('home.chiefOfStaffDigest.queue.toast.started')
      toast.success(message, {
        action: {
          label: t('home.chiefOfStaffDigest.queue.toast.open'),
          onClick: () =>
            open(
              rowView({
                threadId: opens === 'thread' ? row.threadId : null,
                requestId,
              })
            ),
        },
      })
    } catch (e) {
      toast.error(t('home.chiefOfStaffDigest.queue.toast.approveFailed'), {
        description: e instanceof Error ? e.message : String(e),
      })
    }
  }

  const dismissRow = async (row: QueueRow) => {
    try {
      await dismiss.mutateAsync(row.id)
    } catch (e) {
      toast.error(t('home.chiefOfStaffDigest.queue.toast.dismissFailed'), {
        description: e instanceof Error ? e.message : String(e),
      })
    }
  }

  return {
    busy: approve.isPending || dismiss.isPending,
    confirming,
    cancelConfirm: () => setConfirming(null),
    /** Approves at once, or asks first when screening is suspicious */
    requestApprove: (row: QueueRow) =>
      row.needsConfirm ? setConfirming(row) : void approveRow(row),
    approveConfirmed: () => confirming && void approveRow(confirming),
    dismiss: (row: QueueRow) => void dismissRow(row),
  }
}
