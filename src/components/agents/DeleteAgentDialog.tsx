import { useState } from 'react'
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
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import { useDeleteAgent } from '@/hooks/useAgentEditor'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * Deleting asks for the agent's name (never its folder ID): it moves to
 * the trash and stops taking requests. Afterwards the Agents list opens.
 */
export function DeleteAgentDialog({
  agent,
  open,
  onOpenChange,
}: {
  agent: { id: string; name: string }
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const remove = useDeleteAgent()
  const [typed, setTyped] = useState('')
  const matches = typed.trim() === agent.name.trim()

  const confirm = () =>
    remove.mutate(agent.id, {
      onSuccess: () => {
        onOpenChange(false)
        toast.success(t('agentPage.delete.done', { name: agent.name }))
        useNavigationStore.getState().navigate({ kind: 'agents' })
      },
      onError: error =>
        toast.error(t('agentPage.delete.failed'), {
          description: error instanceof Error ? error.message : String(error),
        }),
    })

  return (
    <AlertDialog
      open={open}
      onOpenChange={next => {
        if (!next) setTyped('')
        onOpenChange(next)
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('agentPage.delete.title', { name: agent.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t('agentPage.delete.description', { name: agent.name })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-1.5">
          <Label htmlFor="delete-agent-name">
            {t('agentPage.delete.typeName', { name: agent.name })}
          </Label>
          <Input
            id="delete-agent-name"
            value={typed}
            autoComplete="off"
            onChange={event => setTyped(event.target.value)}
          />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('agentPage.delete.cancel')}</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={!matches || remove.isPending}
            onClick={confirm}
          >
            {remove.isPending && <Spinner />}
            {t('agentPage.delete.confirm')}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
