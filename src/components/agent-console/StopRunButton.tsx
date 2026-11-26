import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Square } from 'lucide-react'
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
import { cancelQueuedRun, stopRun } from '@/lib/agent-actions'
import type { RunDetail } from '@/lib/tauri-bindings'
import { stopNeedsConfirm } from './run-logs-model'

/**
 * A button that runs `onConfirm` at once, or first asks in a dialog when
 * `confirm` says what could happen
 */
export function ConfirmingButton({
  confirm,
  onConfirm,
  children,
  className,
  variant = 'outline',
}: {
  confirm: { title: string; description: string; action: string } | null
  onConfirm: () => void
  children: React.ReactNode
  className?: string
  variant?: 'outline' | 'destructive'
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button
        size="sm"
        variant={variant}
        className={className}
        onClick={() => (confirm ? setOpen(true) : onConfirm())}
      >
        {children}
      </Button>
      {confirm && (
        <AlertDialog open={open} onOpenChange={setOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{confirm.title}</AlertDialogTitle>
              <AlertDialogDescription>
                {confirm.description}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>
                {t('runDetail.confirm.cancel')}
              </AlertDialogCancel>
              <AlertDialogAction onClick={onConfirm}>
                {confirm.action}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}
    </>
  )
}

/** Stop while the run is going or waiting; asks first if it did outward work */
export function StopRunButton({ detail }: { detail: RunDetail }) {
  const { t } = useTranslation()
  const { run } = detail
  const stop = () => {
    const done =
      run.status === 'queued' ? cancelQueuedRun(run.id) : stopRun(run.id)
    void done
  }
  return (
    <ConfirmingButton
      variant="destructive"
      onConfirm={stop}
      confirm={
        run.status !== 'queued' && stopNeedsConfirm(detail)
          ? {
              title: t('runDetail.stop.confirmTitle'),
              description: t('runDetail.stop.confirmDescription'),
              action: t('runDetail.stop.confirmAction'),
            }
          : null
      }
    >
      <Square />
      {t('runDetail.stop.button')}
    </ConfirmingButton>
  )
}
