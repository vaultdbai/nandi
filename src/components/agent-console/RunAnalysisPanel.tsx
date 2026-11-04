/**
 * "Ask about this run": a generated summary (on first open), then questions
 * answered from the run's own record, each with the sources it cites.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Activity,
  FileText,
  GitCommitHorizontal,
  KeyRound,
  MessageSquare,
  Send,
  Sparkles,
  TriangleAlert,
} from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { analysisErrorMessage, useRunAnalysis } from '@/hooks/useRunAnalysis'
import type { AnalysisResponse, SourceRef } from '@/lib/bindings'
import { openSettings } from '@/lib/settings-nav'
import { cn } from '@/lib/utils'
import { MarkdownContent } from './MarkdownContent'
import { formatCitations, NO_API_KEY_ERROR } from './run-detail-model'

const SOURCE_ICONS: Record<string, typeof Activity> = {
  event: Activity,
  message: MessageSquare,
  artifact: FileText,
  revision: GitCommitHorizontal,
}

function AnalysisError({ error }: { error: unknown }) {
  const { t } = useTranslation()
  const message = analysisErrorMessage(error)
  const noKey = message === NO_API_KEY_ERROR
  return (
    <div
      role="alert"
      className={cn(
        'flex flex-col gap-2 rounded-lg border px-3 py-2 text-sm',
        noKey
          ? 'border-warning/30 bg-warning/5'
          : 'border-destructive/30 bg-destructive/5 text-destructive'
      )}
    >
      <div className="flex items-start gap-2">
        {noKey ? (
          <KeyRound className="mt-0.5 size-4 shrink-0 text-warning" />
        ) : (
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
        )}
        <p className="min-w-0 flex-1">
          {noKey ? t('runDetail.analysis.noApiKey') : message}
        </p>
      </div>
      {noKey && (
        <Button
          size="sm"
          variant="outline"
          className="h-7 self-start"
          onClick={() => openSettings('models')}
        >
          {t('runDetail.analysis.openSettings')}
        </Button>
      )}
    </div>
  )
}

export function SourceList({
  sources,
  onOpenSource,
}: {
  sources: SourceRef[]
  onOpenSource?: (source: SourceRef) => void
}) {
  const { t } = useTranslation()
  if (sources.length === 0) return null
  return (
    <section
      aria-label={t('runDetail.analysis.sources')}
      className="flex flex-col gap-1"
    >
      <h4 className="text-xs font-medium text-muted-foreground">
        {t('runDetail.analysis.sources')}
      </h4>
      <ul className="flex flex-col gap-0.5">
        {sources.map(source => {
          const Icon = SOURCE_ICONS[source.kind] ?? FileText
          return (
            <li key={`${source.kind}:${source.id}`}>
              <button
                type="button"
                disabled={!onOpenSource}
                onClick={() => onOpenSource?.(source)}
                className="flex w-full items-center gap-2 rounded px-1 py-0.5 text-start text-xs hover:bg-muted disabled:hover:bg-transparent"
                title={`${source.kind}:${source.id}`}
              >
                <Icon className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate">{source.label}</span>
                <code
                  dir="ltr"
                  className="ms-auto shrink-0 font-mono text-[10px] text-muted-foreground"
                >
                  {source.kind}:{source.id}
                </code>
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}

function KeyRisks({ risks }: { risks: string[] }) {
  const { t } = useTranslation()
  if (risks.length === 0) return null
  return (
    <section className="flex flex-col gap-1">
      <h4 className="text-xs font-medium text-muted-foreground">
        {t('runDetail.analysis.keyRisks')}
      </h4>
      <ul className="list-disc ps-5 text-sm">
        {risks.map(risk => (
          <li key={risk}>
            <MarkdownContent content={formatCitations(risk)} />
          </li>
        ))}
      </ul>
    </section>
  )
}

export function AnswerCard({
  answer,
  onOpenSource,
}: {
  answer: AnalysisResponse
  onOpenSource?: (source: SourceRef) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-2">
      <div className="self-end rounded-lg bg-primary/10 px-3 py-2 text-sm">
        {answer.question}
      </div>
      <div className="flex flex-col gap-3 rounded-lg border bg-card p-3">
        <div className="text-sm">
          <MarkdownContent content={formatCitations(answer.summary)} />
        </div>
        <KeyRisks risks={answer.keyRisks} />
        <SourceList sources={answer.sources} onOpenSource={onOpenSource} />
        {answer.cached && (
          <span className="text-[10px] text-muted-foreground">
            {t('runDetail.analysis.cached')}
          </span>
        )}
      </div>
    </div>
  )
}

export function RunAnalysisPanel({
  runId,
  onOpenSource,
}: {
  runId: string
  /** A cited source was clicked (e.g. switch to the matching tab) */
  onOpenSource?: (source: SourceRef) => void
}) {
  const { t } = useTranslation()
  const { summary, answers, ask } = useRunAnalysis(runId)
  const [question, setQuestion] = useState('')

  const submit = () => {
    const text = question.trim()
    if (!text || ask.isPending) return
    ask.mutate(text, { onSuccess: () => setQuestion('') })
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
        <Sparkles className="size-4 text-primary" />
        <h2 className="text-sm font-semibold">
          {t('runDetail.analysis.title')}
        </h2>
        <Badge variant="secondary" className="ms-auto">
          {t('runDetail.analysis.badge')}
        </Badge>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-4 p-3">
          <section
            aria-label={t('runDetail.analysis.summaryTitle')}
            className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3"
          >
            <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t('runDetail.analysis.summaryTitle')}
            </h3>
            {summary.isPending && summary.fetchStatus !== 'idle' ? (
              <div
                className="flex flex-col gap-2"
                aria-busy="true"
                aria-label={t('runDetail.analysis.summarizing')}
              >
                <Skeleton className="h-3 w-full" />
                <Skeleton className="h-3 w-5/6" />
                <Skeleton className="h-3 w-2/3" />
              </div>
            ) : summary.error ? (
              <AnalysisError error={summary.error} />
            ) : summary.data ? (
              <div className="text-sm">
                <MarkdownContent content={formatCitations(summary.data)} />
              </div>
            ) : null}
          </section>

          {answers.map(answer => (
            <AnswerCard
              key={answer.question}
              answer={answer}
              onOpenSource={onOpenSource}
            />
          ))}

          {ask.isPending && (
            <div className="flex flex-col gap-2">
              <div className="self-end rounded-lg bg-primary/10 px-3 py-2 text-sm">
                {ask.variables}
              </div>
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <Spinner className="size-3" />
                {t('runDetail.analysis.thinking')}
              </div>
            </div>
          )}

          {ask.error && !ask.isPending && <AnalysisError error={ask.error} />}
        </div>
      </ScrollArea>

      <form
        className="flex shrink-0 flex-col gap-2 border-t p-3"
        onSubmit={event => {
          event.preventDefault()
          submit()
        }}
      >
        <Textarea
          value={question}
          onChange={event => setQuestion(event.target.value)}
          onKeyDown={event => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault()
              submit()
            }
          }}
          placeholder={t('runDetail.analysis.placeholder')}
          aria-label={t('runDetail.analysis.placeholder')}
          rows={2}
          className="min-h-14 resize-none text-sm"
        />
        <Button
          type="submit"
          size="sm"
          className="self-end"
          disabled={!question.trim() || ask.isPending}
        >
          <Send />
          {t('runDetail.analysis.send')}
        </Button>
      </form>
    </div>
  )
}
