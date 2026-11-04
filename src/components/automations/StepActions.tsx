import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, X } from 'lucide-react'
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
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { useCompleteManualStep } from '@/hooks/useWork'
import { queryClient } from '@/lib/query-client'
import { approveWorkflowStep } from '@/lib/workflow-actions'
import { stepLabel } from './automations-model'

/**
 * Approve (no confirmation: the step says what it will do) and Reject
 * (confirmed: the run stops there), for a step waiting on the user
 */
export function StepApprovalButtons({
  workflowId,
  runId,
  stepId,
}: {
  workflowId: string
  runId: string
  stepId: string
}) {
  const { t } = useTranslation()
  const [rejecting, setRejecting] = useState(false)
  const [busy, setBusy] = useState(false)

  const decide = async (decision: 'approved' | 'rejected') => {
    setBusy(true)
    await approveWorkflowStep(workflowId, runId, stepId, decision)
    setBusy(false)
  }

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button size="sm" disabled={busy} onClick={() => void decide('approved')}>
        {busy ? <Spinner /> : <Check />}
        {t('workflows.approve')}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => setRejecting(true)}
      >
        <X />
        {t('workflows.reject')}
      </Button>
      <AlertDialog open={rejecting} onOpenChange={setRejecting}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('automations.reject.title', { step: stepLabel(stepId) })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('automations.reject.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('automations.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void decide('rejected')}>
              {t('workflows.reject')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** Mark done for a manual step, with an optional note; no confirmation */
export function MarkStepDoneButton({
  workflowId,
  runId,
  stepId,
}: {
  workflowId: string
  runId: string
  stepId: string
}) {
  const { t } = useTranslation()
  const complete = useCompleteManualStep()
  const [open, setOpen] = useState(false)
  const [note, setNote] = useState('')

  const finish = () =>
    complete.mutate(
      { runId, stepId, output: note.trim() || null },
      {
        onSuccess: () => {
          setOpen(false)
          setNote('')
          void queryClient.invalidateQueries({
            queryKey: ['workflow', workflowId],
          })
        },
        onError: error =>
          toast.error(error instanceof Error ? error.message : String(error)),
      }
    )

  if (!open) {
    return (
      <Button size="sm" className="self-start" onClick={() => setOpen(true)}>
        <Check />
        {t('automations.markDone')}
      </Button>
    )
  }
  return (
    <div className="flex flex-col gap-2 rounded-md border p-3">
      <Label htmlFor={`done-note-${stepId}`}>{t('automations.doneNote')}</Label>
      <Textarea
        id={`done-note-${stepId}`}
        dir="auto"
        value={note}
        onChange={event => setNote(event.target.value)}
      />
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={() => setOpen(false)}>
          {t('automations.cancel')}
        </Button>
        <Button size="sm" disabled={complete.isPending} onClick={finish}>
          {complete.isPending && <Spinner />}
          {t('automations.markDone')}
        </Button>
      </div>
    </div>
  )
}
