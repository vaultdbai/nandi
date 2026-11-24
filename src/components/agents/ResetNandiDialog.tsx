import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useResetDefaultAgent } from '@/hooks/useAgentEditor'

/** What a reset puts back as Nandi ships it, in the order the confirm lists */
const RESET_CHANGES = ['agent', 'rules', 'playbooks', 'triggers'] as const

/**
 * The confirm before Nandi goes back to its defaults (owner decision N-5):
 * it names what changes and what stays, and says the old versions stay in
 * the gold copy's git history. Nothing changes until Reset is pressed.
 */
export function ResetNandiDialog({
  agent,
  open,
  onOpenChange,
}: {
  agent: { name: string }
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const reset = useResetDefaultAgent()

  const confirm = () =>
    reset.mutate(undefined, {
      onSuccess: () => {
        onOpenChange(false)
        toast.success(t('agentPage.reset.done', { name: agent.name }))
      },
      onError: error =>
        toast.error(t('agentPage.reset.failed', { name: agent.name }), {
          description: error instanceof Error ? error.message : String(error),
        }),
    })

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('agentPage.reset.title', { name: agent.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t('agentPage.reset.description')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="list-disc space-y-1 ps-5 text-sm">
          {RESET_CHANGES.map(change => (
            <li key={change}>{t(`agentPage.reset.${change}`)}</li>
          ))}
        </ul>
        <p className="text-sm text-muted-foreground">
          {t('agentPage.reset.keeps')}
        </p>
        <p className="text-sm text-muted-foreground">
          {t('agentPage.reset.history')}
        </p>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('agentPage.reset.cancel')}</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={reset.isPending}
            onClick={confirm}
          >
            {reset.isPending && <Spinner />}
            {t('agentPage.reset.confirm')}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
