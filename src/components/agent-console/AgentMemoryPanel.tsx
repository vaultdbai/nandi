/**
 * The agent's memory.md entries: review, edit, delete, refresh from the
 * latest run, and the "disable memory" toggle.
 * Entries show newest first; each keeps its index in the file.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Brain, History, Pencil, RefreshCw, Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { useAgentMemory } from '@/hooks/useAgentMemory'
import type { MemoryEntry } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'

interface AgentMemoryPanelProps {
  agentId: string | undefined
}

export function AgentMemoryPanel({ agentId }: AgentMemoryPanelProps) {
  const { t } = useTranslation()
  const memory = useAgentMemory(agentId)
  const { enabled, refresh, setEnabled } = memory

  if (!agentId) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Brain />
          </EmptyMedia>
          <EmptyDescription>{t('agentMemory.noAgent')}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const memoryOn = enabled.data ?? true

  const handleRefresh = () => {
    refresh.mutate(undefined, {
      onSuccess: () => toast.success(t('agentMemory.refreshed')),
      onError: error =>
        toast.error(t('agentMemory.refreshFailed'), {
          description: error.message,
        }),
    })
  }

  const handleToggle = (disabled: boolean) => {
    setEnabled.mutate(!disabled, {
      onError: error =>
        toast.error(t('agentMemory.toggleFailed'), {
          description: error.message,
        }),
    })
  }

  return (
    <ScrollArea className="h-full">
      <div className="flex flex-col gap-4 p-4">
        <section className="flex flex-col gap-3 rounded-lg border p-3">
          <p className="text-xs text-muted-foreground">
            {t('agentMemory.description')}
          </p>
          <div className="flex items-center justify-between gap-2">
            <Label htmlFor="agent-memory-disabled" className="text-sm">
              {t('agentMemory.disable')}
            </Label>
            <Switch
              id="agent-memory-disabled"
              checked={!memoryOn}
              disabled={enabled.isPending || setEnabled.isPending}
              onCheckedChange={handleToggle}
            />
          </div>
          {!memoryOn && (
            <p className="text-xs text-muted-foreground">
              {t('agentMemory.disabledNote')}
            </p>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={refresh.isPending || !memoryOn}
            onClick={handleRefresh}
          >
            {refresh.isPending ? <Spinner /> : <RefreshCw />}
            {t('agentMemory.refresh')}
          </Button>
        </section>

        <EntryList memory={memory} />
      </div>
    </ScrollArea>
  )
}

type Memory = ReturnType<typeof useAgentMemory>

function EntryList({ memory }: { memory: Memory }) {
  const { t } = useTranslation()
  const { entries } = memory

  if (entries.isPending) {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-16 w-full" />
      </div>
    )
  }
  if (entries.isError) {
    return (
      <p className="text-sm text-destructive">
        {t('agentMemory.loadFailed')}: {entries.error.message}
      </p>
    )
  }
  if (entries.data.length === 0) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Brain />
          </EmptyMedia>
          <EmptyTitle className="text-base">
            {t('agentMemory.emptyTitle')}
          </EmptyTitle>
          <EmptyDescription>{t('agentMemory.empty')}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  // Newest first, each with its index in the file
  const rows = entries.data.map((entry, index) => ({ entry, index })).reverse()

  return (
    <ul
      aria-label={t('agentMemory.listLabel')}
      className="flex flex-col divide-y rounded-lg border"
    >
      {rows.map(({ entry, index }) => (
        <EntryRow
          key={`${index}-${entry.createdAt}-${entry.content}`}
          entry={entry}
          index={index}
          memory={memory}
        />
      ))}
    </ul>
  )
}

function EntryRow({
  entry,
  index,
  memory,
}: {
  entry: MemoryEntry
  index: number
  memory: Memory
}) {
  const { t, i18n } = useTranslation()
  const [draft, setDraft] = useState<string | null>(null)
  const { update, remove } = memory
  const editing = draft !== null

  const when =
    entry.createdAt > 0
      ? new Date(entry.createdAt).toLocaleString(i18n.language, {
          dateStyle: 'medium',
          timeStyle: 'short',
        })
      : t('agentMemory.undated')

  const handleSave = () => {
    const content = (draft ?? '').trim()
    if (!content) return
    if (content === entry.content) {
      setDraft(null)
      return
    }
    update.mutate(
      { index, content },
      {
        onSuccess: () => setDraft(null),
        onError: error =>
          toast.error(t('agentMemory.saveFailed'), {
            description: error.message,
          }),
      }
    )
  }

  const handleDelete = () => {
    remove.mutate(index, {
      onError: error =>
        toast.error(t('agentMemory.deleteFailed'), {
          description: error.message,
        }),
    })
  }

  return (
    <li className="flex flex-col gap-2 p-3">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <time
          dateTime={
            entry.createdAt > 0
              ? new Date(entry.createdAt).toISOString()
              : undefined
          }
        >
          {when}
        </time>
        {entry.sourceRun && (
          <button
            type="button"
            className="flex min-w-0 items-center gap-1 rounded px-1 font-mono hover:bg-accent hover:text-foreground"
            title={t('agentMemory.openRun')}
            onClick={() =>
              useNavigationStore.getState().navigate({
                kind: 'run-detail',
                runId: entry.sourceRun ?? '',
              })
            }
          >
            <History className="size-3 shrink-0" />
            <span className="truncate">
              {t('agentMemory.fromRun', { id: entry.sourceRun.slice(0, 8) })}
            </span>
          </button>
        )}
        {!editing && (
          <div className="ms-auto flex shrink-0 gap-1">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t('agentMemory.edit')}
              title={t('agentMemory.edit')}
              onClick={() => setDraft(entry.content)}
            >
              <Pencil />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t('agentMemory.delete')}
              title={t('agentMemory.delete')}
              disabled={remove.isPending}
              onClick={handleDelete}
            >
              <Trash2 />
            </Button>
          </div>
        )}
      </div>
      {editing ? (
        <div className="flex flex-col gap-2">
          <Textarea
            rows={3}
            aria-label={t('agentMemory.editLabel')}
            value={draft}
            onChange={e => setDraft(e.target.value)}
          />
          <div className="flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={() => setDraft(null)}>
              {t('agentMemory.cancel')}
            </Button>
            <Button
              size="sm"
              disabled={!draft.trim() || update.isPending}
              onClick={handleSave}
            >
              {update.isPending && <Spinner />}
              {t('agentMemory.save')}
            </Button>
          </div>
        </div>
      ) : (
        <p className="text-sm break-words whitespace-pre-wrap">
          {entry.content}
        </p>
      )}
    </li>
  )
}
