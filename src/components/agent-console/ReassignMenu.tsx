import { useTranslation } from 'react-i18next'
import { ArrowRightLeft } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  isReassignable,
  taskErrorMessage,
  useReassignCandidates,
  useReassignTask,
} from '@/hooks/useTasks'
import type { TaskRecord } from '@/lib/bindings'
import { toastPersonError } from '@/lib/toast-error'

/**
 * Supervisor mode for a task we're serving: hand it to an online teammate
 * who serves the same capability. Hidden when nobody else could take it.
 */
export function ReassignMenu({ task }: { task: TaskRecord }) {
  const { t } = useTranslation()
  const open = isReassignable(task.state)
  const { data: candidates } = useReassignCandidates(task.id, open)
  const reassign = useReassignTask()

  if (!open || !candidates || candidates.length === 0) return null

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          disabled={reassign.isPending}
          data-testid="reassign-task"
        >
          <ArrowRightLeft />
          {t('tasks.provider.reassign')}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>{t('tasks.provider.reassignTo')}</DropdownMenuLabel>
        {candidates.map(candidate => (
          <DropdownMenuItem
            key={candidate.instanceId}
            onSelect={() =>
              reassign.mutate(
                {
                  taskId: task.id,
                  newProviderInstance: candidate.instanceId,
                },
                {
                  onSuccess: () =>
                    toast.success(
                      t('tasks.provider.reassigned', {
                        name: candidate.displayName,
                      })
                    ),
                  onError: error =>
                    toastPersonError(error, failure =>
                      toast.error(
                        t('tasks.provider.reassignFailed', {
                          error: taskErrorMessage(failure),
                        })
                      )
                    ),
                }
              )
            }
          >
            {candidate.displayName}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
