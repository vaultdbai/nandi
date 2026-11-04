import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
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
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * Save as workflow: the core drafts a workflow file from the Request's
 * plan, the user reads (and may edit) it, and saving validates it like
 * any workflow file. Opens the new workflow.
 */
/** `unwrapResult` throws the error `Result` itself */
function errorText(error: unknown): string {
  if (error && typeof error === 'object' && 'error' in error) {
    return String((error as { error: unknown }).error)
  }
  return String(error)
}

export function SaveAsWorkflowDialog({
  requestId,
  open,
  onOpenChange,
}: {
  requestId: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const draft = useQuery({
    queryKey: ['requests', 'workflow-draft', requestId],
    queryFn: async () =>
      unwrapResult(await client.draftWorkflowFromRequest(requestId)),
    enabled: open,
    retry: false,
    staleTime: 0,
  })
  // The user's edits to the draft (`null`: none yet)
  const [edited, setEdited] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const yaml = edited ?? draft.data ?? null
  const error = saveError ?? (draft.isError ? errorText(draft.error) : null)

  const close = (next: boolean) => {
    if (!next) {
      setEdited(null)
      setSaveError(null)
    }
    onOpenChange(next)
  }

  const save = async () => {
    if (yaml === null) return
    setSaving(true)
    const result = await client.createWorkflow({ yaml })
    setSaving(false)
    if (result.status === 'error') {
      setSaveError(result.error)
      return
    }
    toast.success(t('request.saveWorkflow.saved', { name: result.data.name }))
    close(false)
    useNavigationStore
      .getState()
      .navigate({ kind: 'workflow', workflowId: result.data.id })
  }

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t('request.saveWorkflow.title')}</DialogTitle>
          <DialogDescription>
            {t('request.saveWorkflow.description')}
          </DialogDescription>
        </DialogHeader>
        {yaml === null && !error ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner />
            {t('request.saveWorkflow.drafting')}
          </p>
        ) : (
          yaml !== null && (
            <Textarea
              aria-label={t('request.saveWorkflow.file')}
              dir="ltr"
              spellCheck={false}
              className="min-h-64 font-mono text-xs"
              value={yaml}
              onChange={event => setEdited(event.target.value)}
            />
          )
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => close(false)}>
            {t('request.saveWorkflow.cancel')}
          </Button>
          <Button
            disabled={yaml === null || saving}
            onClick={() => void save()}
          >
            {saving && <Spinner />}
            {t('request.saveWorkflow.save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
