import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CheckCircle2, RotateCcw, Save } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { ClarificationPolicyEditor } from '@/components/agent-console/ClarificationPolicyEditor'
import { errorMessage } from '@/hooks/useDelegation'
import {
  useClarificationPolicy,
  useSetClarificationPolicy,
} from '@/hooks/useClarifications'
import type { ClarificationPolicy } from '@/lib/bindings'

export interface AgentDelegationTabProps {
  agentId: string
}

/**
 * The agent editor's Delegation tab: its clarification policy, read from
 * and saved to the agent's skill.md (`clarification:`) on its own, apart
 * from the rest of the editor's draft.
 */
export function AgentDelegationTab({ agentId }: AgentDelegationTabProps) {
  const { t } = useTranslation()
  const saved = useClarificationPolicy(agentId)
  const save = useSetClarificationPolicy(agentId)
  // `null`: no unsaved edits, show what's saved
  const [draft, setDraft] = useState<ClarificationPolicy | null>(null)
  const [justSaved, setJustSaved] = useState(false)

  const edit = (next: ClarificationPolicy) => {
    setDraft(next)
    setJustSaved(false)
    save.reset()
  }

  const handleSave = () => {
    if (!draft) return
    save.mutate(draft, {
      onSuccess: () => {
        setDraft(null)
        setJustSaved(true)
      },
    })
  }

  return (
    <div className="flex flex-col gap-4" data-testid="agent-delegation-tab">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">{t('tasks.policy.title')}</h3>
        <p className="text-xs text-muted-foreground">
          {t('tasks.policy.description')}
        </p>
      </div>

      {saved.isPending && (
        <div className="flex flex-col gap-3" aria-busy="true">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-2/3" />
        </div>
      )}

      {saved.isError && (
        <Alert variant="destructive">
          <AlertTitle>{t('tasks.policy.loadFailed')}</AlertTitle>
          <AlertDescription>{errorMessage(saved.error)}</AlertDescription>
        </Alert>
      )}

      {saved.isSuccess && (
        <>
          <ClarificationPolicyEditor
            value={draft ?? saved.data}
            onChange={edit}
            disabled={save.isPending}
            idPrefix={`clarification-policy-${agentId}`}
          />
          {save.isError && (
            <Alert variant="destructive">
              <AlertTitle>{t('tasks.policy.saveFailed')}</AlertTitle>
              <AlertDescription>{errorMessage(save.error)}</AlertDescription>
            </Alert>
          )}
          <div className="flex items-center justify-end gap-2">
            {justSaved && draft === null && (
              <span
                role="status"
                className="me-auto flex items-center gap-1 text-xs text-muted-foreground"
              >
                <CheckCircle2 className="size-4" />
                {t('tasks.policy.saved')}
              </span>
            )}
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={draft === null || save.isPending}
              onClick={() => {
                setDraft(null)
                save.reset()
              }}
            >
              <RotateCcw />
              {t('common.reset')}
            </Button>
            <Button
              type="button"
              size="sm"
              disabled={draft === null || save.isPending}
              onClick={handleSave}
            >
              {save.isPending ? <Spinner /> : <Save />}
              {t('tasks.policy.save')}
            </Button>
          </div>
        </>
      )}
    </div>
  )
}
