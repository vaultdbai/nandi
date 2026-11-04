import { useState } from 'react'
import { useTranslation } from 'react-i18next'
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
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { errorMessage, useDelegateTask } from '@/hooks/useDelegation'
import { toastPersonError } from '@/lib/toast-error'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * Sends a plain-words request to whichever provider selection picks for
 * the capability (`target: null`), then opens the task's thread.
 */
export function DelegateCapabilityDialog({
  capabilityId,
  name,
  open,
  onOpenChange,
}: {
  capabilityId: string
  name: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const delegate = useDelegateTask()
  const [text, setText] = useState('')
  const canSend = text.trim() !== '' && !delegate.isPending

  const send = async () => {
    if (!canSend) return
    try {
      const taskId = await delegate.mutateAsync({
        capability: capabilityId,
        input: { text: text.trim() },
        target: null,
      })
      setText('')
      onOpenChange(false)
      toast.success(t('capabilities.delegate.sent', { name }))
      useNavigationStore
        .getState()
        .navigate({ kind: 'delegation-thread', taskId })
    } catch (error) {
      toastPersonError(error, failure =>
        toast.error(
          t('capabilities.delegate.failed', { error: errorMessage(failure) })
        )
      )
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form
          className="flex flex-col gap-4"
          onSubmit={event => {
            event.preventDefault()
            void send()
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {t('capabilities.delegate.title', { name })}
            </DialogTitle>
            <DialogDescription>
              {t('capabilities.delegate.description')}
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2">
            <Label htmlFor={`delegate-${capabilityId}`}>
              {t('capabilities.delegate.request')}
            </Label>
            <Textarea
              id={`delegate-${capabilityId}`}
              value={text}
              rows={4}
              placeholder={t('capabilities.delegate.placeholder')}
              onChange={event => setText(event.target.value)}
            />
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              {t('capabilities.cancel')}
            </Button>
            <Button type="submit" disabled={!canSend}>
              {delegate.isPending && <Spinner />}
              {t('capabilities.delegate.send')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
