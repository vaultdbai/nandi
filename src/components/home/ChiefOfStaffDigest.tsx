import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Sparkles } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { SaveAsWorkflowDialog } from '@/components/requests/SaveAsWorkflowDialog'
import {
  useChiefOfStaffDigest,
  useCompleteWaitingStep,
  useMarkDigestSeen,
} from '@/hooks/useChiefOfStaffDigest'
import { useAlwaysAllow } from '@/hooks/useAutoApproveRules'
import { rawError } from '@/lib/friendly-error'
import { useQueueDecisions } from '@/hooks/useQueueDecisions'
import { useViewOpener } from '@/hooks/useViewOpener'
import { cn } from '@/lib/utils'
import {
  digestRows,
  digestSummary,
  digestTitleKey,
  queueRows,
  rowView,
  type DigestRow,
} from './digest-model'
import {
  AlwaysAllowDialog,
  ApproveSuspiciousDialog,
  QueueRowView,
} from './QueueRowView'

/**
 * Home's "Since you were away": what the Chief of Staff started, updated
 * or finished on its own, and what waits on the user. "Got it" marks it
 * seen; needs-you rows stay until they are done. Rows open the item's
 * Chief of Staff thread, or the Request when there is none. Finished work
 * that recurs offers "Save as workflow": the user checks the draft first.
 */
export function ChiefOfStaffDigest() {
  const { t, i18n } = useTranslation()
  const { open } = useViewOpener()
  const digest = useChiefOfStaffDigest()
  const markSeen = useMarkDigestSeen()
  const completeStep = useCompleteWaitingStep()
  const decisions = useQueueDecisions('thread')
  const alwaysAllow = useAlwaysAllow()
  // The Request whose workflow draft is open
  const [saving, setSaving] = useState<string | null>(null)
  if (!digest.data) return null
  if (digest.data.items.length === 0 && digest.data.queue.length === 0) {
    return null
  }

  const rows = digestRows(digest.data, i18n.language)
  const queue = queueRows(digest.data, i18n.language)
  const summary = digestSummary(digest.data)
    .map(part =>
      t(`home.chiefOfStaffDigest.summary.${part.kind}`, { count: part.count })
    )
    .join(' · ')

  const gotIt = async () => {
    try {
      await markSeen.mutateAsync()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  const markDone = (row: DigestRow) => {
    if (!row.waitingStep) return
    completeStep.mutate(
      { requestId: row.requestId, position: row.waitingStep.position },
      {
        onSuccess: () =>
          toast.success(t('home.chiefOfStaffDigest.markDone.done'), {
            action: {
              label: t('home.chiefOfStaffDigest.queue.toast.open'),
              onClick: () => open(rowView(row)),
            },
          }),
        onError: error =>
          toast.error(t('home.chiefOfStaffDigest.markDone.failed'), {
            description: rawError(error),
          }),
      }
    )
  }

  return (
    <Card data-testid="home-cos-digest">
      <CardHeader className="flex flex-row items-start gap-3">
        <Sparkles
          aria-hidden
          className="mt-1 size-4 shrink-0 text-muted-foreground"
        />
        <div className="min-w-0 flex-1">
          <CardTitle className="text-base">
            {t(digestTitleKey(digest.data))}
          </CardTitle>
          <p className="text-sm text-muted-foreground">{summary}</p>
        </div>
        {rows.length > 0 && (
          <Button
            size="sm"
            variant="outline"
            disabled={markSeen.isPending}
            onClick={() => void gotIt()}
          >
            {t('home.chiefOfStaffDigest.gotIt')}
          </Button>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-1">
        {queue.length > 0 && (
          <section
            data-testid="home-cos-queue"
            aria-label={t('home.chiefOfStaffDigest.queue.title')}
            className="mb-2 flex flex-col gap-2"
          >
            <h3 className="px-2 text-sm font-medium">
              {t('home.chiefOfStaffDigest.queue.title')}
            </h3>
            {queue.map(row => (
              <QueueRowView
                key={row.id}
                row={row}
                busy={decisions.busy || alwaysAllow.busy}
                onApprove={() => decisions.requestApprove(row)}
                onDismiss={() => decisions.dismiss(row)}
                onAlwaysAllow={() => alwaysAllow.ask(row)}
                onOpen={() =>
                  open({ kind: 'intake-thread', conversationId: row.threadId })
                }
              />
            ))}
          </section>
        )}
        {rows.map((row, index) => (
          <DigestRowButton
            // One Request can show twice (waiting on you, and updated)
            key={`${row.kind}-${row.requestId}-${index}`}
            row={row}
            busy={completeStep.isPending}
            onOpen={() => open(rowView(row))}
            onMarkDone={() => markDone(row)}
            onSaveWorkflow={() => setSaving(row.requestId)}
          />
        ))}
      </CardContent>
      {saving && (
        <SaveAsWorkflowDialog
          requestId={saving}
          open
          onOpenChange={next => {
            if (!next) setSaving(null)
          }}
        />
      )}
      <ApproveSuspiciousDialog
        open={decisions.confirming !== null}
        onCancel={decisions.cancelConfirm}
        onApprove={decisions.approveConfirmed}
      />
      <AlwaysAllowDialog
        domain={alwaysAllow.asking?.alwaysAllowDomain ?? null}
        onCancel={alwaysAllow.cancel}
        onConfirm={alwaysAllow.confirm}
      />
    </Card>
  )
}

function DigestRowButton({
  row,
  busy,
  onOpen,
  onMarkDone,
  onSaveWorkflow,
}: {
  row: DigestRow
  busy: boolean
  onOpen: () => void
  onMarkDone: () => void
  onSaveWorkflow: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex min-w-0 items-center gap-2 rounded-md hover:bg-muted">
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 flex-col gap-0.5 rounded-md px-2 py-2 text-start text-sm"
      >
        <span className="flex min-w-0 items-center gap-2">
          <Badge
            variant={row.kind === 'needsYou' ? 'default' : 'secondary'}
            className="shrink-0"
          >
            {t(row.labelKey)}
          </Badge>
          <span className="min-w-0 flex-1 truncate font-medium">
            {row.title}
          </span>
          <span className="shrink-0 text-xs text-muted-foreground">
            {t(row.sourceKey)} · {row.when}
          </span>
        </span>
        {row.text && (
          <span className={cn('truncate ps-2 text-xs text-muted-foreground')}>
            {row.text}
          </span>
        )}
        {row.workflowProposal && (
          <span className="truncate ps-2 text-xs text-muted-foreground">
            {t('request.saveWorkflow.proposal', {
              title: row.workflowProposal.likeTitle,
            })}
          </span>
        )}
      </button>
      {row.workflowProposal && (
        <Button
          size="sm"
          variant="outline"
          className="me-2 shrink-0"
          data-testid="home-cos-save-workflow"
          onClick={onSaveWorkflow}
        >
          {t('request.saveWorkflow.button')}
        </Button>
      )}
      {row.waitingStep && (
        <Button
          size="sm"
          variant="outline"
          className="me-2 shrink-0"
          disabled={busy}
          title={row.waitingStep.label}
          aria-label={t('home.chiefOfStaffDigest.markDone.aria', {
            step: row.waitingStep.label,
          })}
          onClick={onMarkDone}
        >
          {t('home.chiefOfStaffDigest.markDone.label')}
        </Button>
      )}
    </div>
  )
}
