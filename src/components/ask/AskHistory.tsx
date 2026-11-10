import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight, History } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { useAskHistory } from '@/hooks/useAsk'
import type { AskHistoryEntry } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import { ArtifactLabel } from './EditPreview'

/**
 * Recent asks, collapsed by default (Studio and the companion). Each opens
 * to what came back: the answer, the plan's steps, the edit's artifact, or
 * the question asked back.
 */
export function AskHistory({
  limit = 10,
  className,
}: {
  limit?: number
  className?: string
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const { data: entries = [] } = useAskHistory(limit)

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className={cn('flex flex-col gap-1.5', className)}
    >
      <CollapsibleTrigger className="flex items-center gap-1.5 self-start text-xs font-medium text-muted-foreground hover:text-foreground">
        <ChevronRight
          aria-hidden
          className={cn('size-3.5 transition-transform', open && 'rotate-90')}
        />
        <History aria-hidden className="size-3.5" />
        {t('ask.history.title', { count: entries.length })}
      </CollapsibleTrigger>
      <CollapsibleContent>
        {entries.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            {t('ask.history.empty')}
          </p>
        ) : (
          <ul
            aria-label={t('ask.history.label')}
            className="flex max-h-64 flex-col gap-1 overflow-y-auto"
          >
            {entries.map(entry => (
              <HistoryEntry key={entry.id} entry={entry} />
            ))}
          </ul>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}

function HistoryEntry({ entry }: { entry: AskHistoryEntry }) {
  const { t, i18n } = useTranslation()
  const [open, setOpen] = useState(false)
  const { response } = entry

  return (
    <li className="rounded-md border px-2 py-1.5">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(value => !value)}
        className="flex w-full items-center gap-2 text-start"
      >
        <span className="min-w-0 flex-1 truncate text-sm">
          {entry.requestText}
        </span>
        <Badge variant="outline" className="shrink-0 text-[10px]">
          {t(`ask.history.kind.${response.kind}`)}
        </Badge>
        <span className="shrink-0 text-xs text-muted-foreground">
          {formatRelativeTime(entry.at, i18n.language)}
        </span>
      </button>
      {open && (
        <div className="mt-1.5 text-xs text-muted-foreground">
          {response.kind === 'question' && (
            <MarkdownContent content={response.data.answer} />
          )}
          {response.kind === 'action' &&
            (response.data.rejection ?? (
              <ol className="list-decimal ps-4">
                {response.data.steps.map((step, index) => (
                  <li key={index}>{step.description}</li>
                ))}
              </ol>
            ))}
          {response.kind === 'edit' && (
            <span>
              <ArtifactLabel artifact={response.data.artifact} />
              {response.data.versionBump &&
                ` · ${t('ask.edit.bump', { bump: response.data.versionBump })}`}
            </span>
          )}
          {response.kind === 'clarification' && response.data}
          {response.kind === 'help' &&
            (response.data.topic
              ? t(`help.topics.${response.data.topic.id}.title`)
              : t('help.noMatch'))}
        </div>
      )}
    </li>
  )
}
