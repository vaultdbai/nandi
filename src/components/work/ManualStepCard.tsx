import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CalendarClock, Paperclip, UserRoundCheck } from 'lucide-react'
import { toast } from 'sonner'
import { AttachmentChip } from '@/components/agent-console/AttachmentChip'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { useCompleteManualStep } from '@/hooks/useWork'
import { client } from '@/lib/client'
import type { AttachmentInfo, PendingManualStep } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

export interface ManualStepCardProps {
  step: PendingManualStep
  /** For tests: "now" for the overdue check */
  now?: number
}

/**
 * A workflow step waiting for a person (wave EF): what to do, when it's
 * due, and a reply (text and files) that completes it and resumes the run
 */
export function ManualStepCard({ step, now }: ManualStepCardProps) {
  const { t, i18n } = useTranslation()
  const [output, setOutput] = useState('')
  const [files, setFiles] = useState<AttachmentInfo[]>([])
  // When the card appeared: the overdue check doesn't tick
  const [shownAt] = useState(Date.now)
  const complete = useCompleteManualStep()
  const overdue = step.deadline !== null && step.deadline < (now ?? shownAt)

  const attach = async () => {
    const result = await client.pickAttachments('file')
    if (result.status === 'ok') {
      setFiles(current => [...current, ...result.data])
    } else {
      toast.error(t('work.manual.attachFailed', { error: result.error }))
    }
  }

  const submit = () =>
    complete.mutate(
      {
        runId: step.runId,
        stepId: step.stepId,
        output: output.trim() || null,
        attachmentIds: files.map(file => file.id),
      },
      {
        onSuccess: () => {
          toast.success(t('work.manual.completed'))
          setOutput('')
          setFiles([])
        },
        onError: error =>
          toast.error(
            t('work.manual.failed', {
              error: error instanceof Error ? error.message : String(error),
            })
          ),
      }
    )

  return (
    <article
      className="flex flex-col gap-3 rounded-lg border border-warning/30 bg-warning/5 p-3"
      data-testid={`manual-step-${step.runId}-${step.stepId}`}
      aria-label={t('work.manual.label', {
        step: step.stepId,
        workflow: step.workflowName,
      })}
    >
      <header className="flex flex-wrap items-center gap-2">
        <UserRoundCheck aria-hidden className="size-4 text-warning" />
        <span className="text-sm font-medium">{step.workflowName}</span>
        <Badge variant="outline">{step.stepId}</Badge>
        {step.deadline !== null && (
          <Badge
            variant={overdue ? 'destructive' : 'secondary'}
            className="gap-1"
          >
            <CalendarClock aria-hidden className="size-3" />
            {t(overdue ? 'work.manual.overdue' : 'work.manual.due', {
              when: formatRelativeTime(step.deadline, i18n.language),
            })}
          </Badge>
        )}
        <span className="ms-auto text-xs text-muted-foreground">
          {t('work.manual.since', {
            when: formatRelativeTime(step.readyAt, i18n.language),
          })}
        </span>
      </header>
      <p className="text-sm whitespace-pre-wrap" dir="auto">
        {step.instructions}
      </p>
      <Textarea
        value={output}
        onChange={event => setOutput(event.target.value)}
        placeholder={t('work.manual.outputPlaceholder')}
        aria-label={t('work.manual.output')}
        rows={2}
      />
      {files.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {files.map(file => (
            <AttachmentChip
              key={file.id}
              attachment={file}
              onRemove={id =>
                setFiles(current => current.filter(f => f.id !== id))
              }
            />
          ))}
        </div>
      )}
      <div className="flex items-center gap-2">
        {client.supports.attachments && (
          <Button variant="ghost" size="sm" onClick={() => void attach()}>
            <Paperclip />
            {t('work.manual.attach')}
          </Button>
        )}
        <Button
          size="sm"
          className="ms-auto"
          disabled={complete.isPending}
          onClick={submit}
        >
          {t('work.manual.complete')}
        </Button>
      </div>
    </article>
  )
}
