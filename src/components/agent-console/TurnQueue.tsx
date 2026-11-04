import { useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { GripVertical, ListOrdered, Square, X } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Spinner } from '@/components/ui/spinner'
import {
  cancelQueuedRun,
  cancelTurn,
  reprioritizeQueuedRun,
} from '@/lib/agent-actions'
import { formatRelativeTime } from '@/lib/format-time'
import { priorityForMove } from '@/lib/queue-order'
import { cn } from '@/lib/utils'
import type { QueueEntry, RunSummary, SourceType } from '@/lib/tauri-bindings'
import { useAgentQueue, useCurrentTurn } from '@/store/agent-store'

const PREVIEW_CHARS = 60

const preview = (text: string) => {
  const line = text.replace(/\s+/g, ' ').trim()
  return line.length > PREVIEW_CHARS ? `${line.slice(0, PREVIEW_CHARS)}…` : line
}

/**
 * The strip above an agent's chat: what it's working on, how many
 * requests wait behind it, and the panel to manage them. Hidden when the
 * agent is idle with nothing queued. Reads only the store, which
 * `agent://queue_changed` keeps live.
 */
export function TurnQueueStrip({
  agentId,
  agentName,
}: {
  agentId: string
  agentName: string
}) {
  const { t } = useTranslation()
  const current = useCurrentTurn(agentId)
  const queue = useAgentQueue(agentId)
  const [open, setOpen] = useState(false)

  if (!current && queue.length === 0) return null
  const waiting = current?.status === 'awaitingApproval'

  return (
    <>
      <div className="flex h-8 shrink-0 items-center gap-2 border-b bg-muted/30 px-4 text-xs text-muted-foreground">
        {current && !waiting && <Spinner className="size-3" />}
        <span className="min-w-0 truncate">
          {current
            ? t(
                waiting
                  ? 'agentConsole.queue.waitingOn'
                  : 'agentConsole.queue.workingOn',
                { input: preview(current.input) }
              )
            : t('agentConsole.queue.starting')}
        </span>
        {queue.length > 0 && (
          <Button
            variant="outline"
            size="sm"
            className="h-5 shrink-0 rounded-full px-2 text-[11px]"
            onClick={() => setOpen(true)}
            aria-label={t('agentConsole.queue.open')}
          >
            {t('agentConsole.queue.queuedCount', { count: queue.length })}
          </Button>
        )}
        <div className="ms-auto flex shrink-0 items-center gap-1">
          {queue.length === 0 && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-6"
              onClick={() => setOpen(true)}
              aria-label={t('agentConsole.queue.open')}
              title={t('agentConsole.queue.open')}
            >
              <ListOrdered className="size-3" />
            </Button>
          )}
          {current && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-xs"
              onClick={() => void cancelTurn(agentId, current.id)}
            >
              <Square className="size-3" />
              {t('agentConsole.step.stop')}
            </Button>
          )}
        </div>
      </div>

      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent className="flex w-full flex-col gap-0 sm:max-w-md">
          <SheetHeader className="border-b">
            <SheetTitle>{t('agentConsole.queue.title')}</SheetTitle>
            <SheetDescription>
              {t('agentConsole.queue.description', { agentName })}
            </SheetDescription>
          </SheetHeader>
          <QueuePanel agentId={agentId} current={current} queue={queue} />
        </SheetContent>
      </Sheet>
    </>
  )
}

function QueuePanel({
  agentId,
  current,
  queue,
}: {
  agentId: string
  current: RunSummary | null
  queue: QueueEntry[]
}) {
  const { t, i18n } = useTranslation()
  const [dragging, setDragging] = useState<number | null>(null)
  const [over, setOver] = useState<number | null>(null)

  const move = (from: number, to: number) => {
    const entry = queue[from]
    const priority = priorityForMove(queue, from, to)
    if (entry && priority !== null && priority !== entry.priority) {
      void reprioritizeQueuedRun(entry.runId, priority)
    }
  }

  const onHandleKey = (event: KeyboardEvent, index: number) => {
    if (!event.altKey) return
    if (event.key === 'ArrowUp' && index > 0) {
      event.preventDefault()
      move(index, index - 1)
    } else if (event.key === 'ArrowDown' && index < queue.length - 1) {
      event.preventDefault()
      move(index, index + 1)
    }
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-3">
      {current && (
        <div className="mb-3 rounded-md border bg-muted/40 p-3">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            {current.status === 'awaitingApproval' ? (
              <Badge variant="outline">
                {t('agentConsole.step.awaitingApproval')}
              </Badge>
            ) : (
              <>
                <Spinner className="size-3" />
                <span>{t('agentConsole.queue.current')}</span>
              </>
            )}
            <span className="truncate">
              {sourceName(t, current.sourceType, current.sourceLabel)}
            </span>
            <Button
              variant="outline"
              size="sm"
              className="ms-auto h-6 px-2 text-xs"
              onClick={() => void cancelTurn(agentId, current.id)}
            >
              <Square className="size-3" />
              {t('agentConsole.step.stop')}
            </Button>
          </div>
          <p className="mt-2 line-clamp-2 text-sm">{current.input}</p>
        </div>
      )}

      {queue.length === 0 ? (
        <p className="px-1 py-6 text-center text-sm text-muted-foreground">
          {t('agentConsole.queue.empty')}
        </p>
      ) : (
        <ol
          aria-label={t('agentConsole.queue.title')}
          className="flex flex-col gap-1.5"
        >
          {queue.map((entry, index) => (
            <li
              key={entry.runId}
              draggable
              onDragStart={event => {
                event.dataTransfer.effectAllowed = 'move'
                setDragging(index)
              }}
              onDragOver={event => {
                if (dragging === null) return
                event.preventDefault()
                setOver(index)
              }}
              onDragLeave={() => setOver(o => (o === index ? null : o))}
              onDrop={event => {
                event.preventDefault()
                if (dragging !== null) move(dragging, index)
                setDragging(null)
                setOver(null)
              }}
              onDragEnd={() => {
                setDragging(null)
                setOver(null)
              }}
              className={cn(
                'flex items-start gap-2 rounded-md border bg-background p-2 transition-colors',
                dragging === index && 'opacity-50',
                over === index && dragging !== index && 'border-ring'
              )}
            >
              <button
                type="button"
                className="mt-0.5 cursor-grab rounded text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring active:cursor-grabbing"
                aria-label={t('agentConsole.queue.reorder')}
                title={t('agentConsole.queue.reorder')}
                onKeyDown={event => onHandleKey(event, index)}
              >
                <GripVertical className="size-4" />
              </button>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  <span className="truncate font-medium text-foreground">
                    {sourceName(t, entry.sourceType, entry.sourceLabel)}
                  </span>
                  <span className="shrink-0">
                    {t('agentConsole.queue.priority', {
                      priority: entry.priority,
                    })}
                  </span>
                  <span className="ms-auto shrink-0">
                    {formatRelativeTime(entry.createdAt, i18n.language)}
                  </span>
                </div>
                <p className="mt-1 line-clamp-2 text-sm break-words">
                  {preview(entry.message) ||
                    entry.attachments.map(a => a.name).join(', ')}
                </p>
              </div>
              <Button
                variant="ghost"
                size="icon-sm"
                className="size-6 shrink-0"
                onClick={() => void cancelQueuedRun(entry.runId)}
                aria-label={t('agentConsole.queue.remove')}
                title={t('agentConsole.queue.remove')}
              >
                <X className="size-3.5" />
              </Button>
            </li>
          ))}
        </ol>
      )}
    </div>
  )
}

/** "You" for the user's own messages, else the source's label */
function sourceName(
  t: ReturnType<typeof useTranslation>['t'],
  sourceType: SourceType,
  label: string | null
): string {
  if (sourceType === 'user') return t('agentConsole.queue.source.user')
  return label ?? t(`agentConsole.queue.source.${sourceType}`)
}
