import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { runDurationMs, useRuns } from '@/hooks/useRuns'
import { client } from '@/lib/client'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import type { RunSummary } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore, type View } from '@/store/navigation-store'
import type { AgentInfo } from '@/types/agent'
import { runOrigin } from './agent-page-model'

const PAGE = 25
const STATUSES = [
  'running',
  'awaitingApproval',
  'completed',
  'failed',
  'cancelled',
] as const

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

/** Opens the Request a run was part of (or its workflow run) */
async function openOrigin(run: RunSummary, notFound: string) {
  const origin = runOrigin(run)
  if (origin.kind === 'workflow' && origin.workflowRunId) {
    navigate({ kind: 'work-item', id: origin.workflowRunId })
    return
  }
  const result = await client.requestForRun(run.id)
  if (result.status === 'ok' && result.data) {
    navigate({ kind: 'request', requestId: result.data })
  } else {
    toast.info(notFound)
  }
}

/** "Part of": the Request, workflow or trigger a run belongs to, or Chat */
export function RunOriginLink({ run }: { run: RunSummary }) {
  const { t } = useTranslation()
  const origin = runOrigin(run)
  if (origin.kind === 'chat') {
    return (
      <span className="text-muted-foreground">{t('agentPage.runs.chat')}</span>
    )
  }
  const label =
    origin.label ||
    t(
      origin.kind === 'workflow'
        ? 'agentPage.runs.workflow'
        : origin.kind === 'request'
          ? 'agentPage.runs.request'
          : 'agentPage.runs.trigger'
    )
  if (origin.kind === 'trigger') {
    return <span className="truncate text-muted-foreground">{label}</span>
  }
  return (
    <button
      type="button"
      className="truncate text-start underline-offset-2 hover:underline"
      onClick={event => {
        // The row itself opens the run
        event.stopPropagation()
        void openOrigin(run, t('agentPage.runs.requestGone'))
      }}
    >
      {label}
    </button>
  )
}

/** One run: when, what it was asked, what it was part of, how it ended */
export function AgentRunRow({ run }: { run: RunSummary }) {
  const { t, i18n } = useTranslation()
  const duration = runDurationMs(run)
  return (
    <li
      className="grid cursor-pointer grid-cols-[6rem_minmax(0,1fr)] items-center gap-x-4 gap-y-0.5 rounded-md px-2 py-2 text-sm hover:bg-accent/40 md:grid-cols-[7rem_minmax(0,2fr)_minmax(0,1.4fr)_9rem]"
      onClick={() => navigate({ kind: 'run-detail', runId: run.id })}
      onKeyDown={event => {
        if (event.key === 'Enter')
          navigate({ kind: 'run-detail', runId: run.id })
      }}
      role="link"
      tabIndex={0}
    >
      <span className="text-xs text-muted-foreground">
        {formatRelativeTime(run.startedAt, i18n.language)}
      </span>
      <span dir="auto" className="truncate">
        {run.input.split('\n')[0] || t('agentPage.runs.noInput')}
      </span>
      <span className="col-start-2 truncate text-xs md:col-start-auto md:text-sm">
        <RunOriginLink run={run} />
      </span>
      <span
        className={cn(
          'col-start-2 text-xs md:col-start-auto md:text-end',
          run.status === 'failed' && 'text-destructive',
          run.status === 'awaitingApproval' && 'text-warning'
        )}
      >
        {t(`runs.status.${run.status}`)}
        {duration !== null && ` · ${formatDuration(duration)}`}
      </span>
    </li>
  )
}

/** Every run of this agent, newest first; a row opens Run Detail */
export function AgentRunsTab({
  agent,
  onChat,
}: {
  agent: AgentInfo
  onChat: () => void
}) {
  const { t } = useTranslation()
  const [status, setStatus] = useState('')
  const [search, setSearch] = useState('')
  const [pages, setPages] = useState(1)
  const filtered = status !== '' || search.trim() !== ''
  const runs = useRuns(
    {
      agentId: agent.id,
      status: status || null,
      search: search.trim() || null,
    },
    0,
    PAGE * pages
  )
  const rows = runs.data?.rows ?? []
  const total = runs.data?.total ?? 0

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={search}
          onChange={event => setSearch(event.target.value)}
          placeholder={t('agentPage.runs.search')}
          aria-label={t('agentPage.runs.search')}
          className="max-w-xs"
        />
        <NativeSelect
          aria-label={t('agentPage.runs.status')}
          value={status}
          onChange={event => setStatus(event.target.value)}
        >
          <NativeSelectOption value="">
            {t('agentPage.runs.anyStatus')}
          </NativeSelectOption>
          {STATUSES.map(value => (
            <NativeSelectOption key={value} value={value}>
              {t(`runs.status.${value}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      {runs.isPending ? (
        <div className="space-y-2">
          {[0, 1, 2].map(i => (
            <Skeleton key={i} className="h-10" />
          ))}
        </div>
      ) : rows.length === 0 ? (
        <Empty className="py-12">
          <EmptyHeader>
            <EmptyTitle>
              {filtered
                ? t('agentPage.runs.noMatch')
                : t('agentPage.runs.empty')}
            </EmptyTitle>
          </EmptyHeader>
          <EmptyContent>
            {filtered ? (
              <Button
                variant="outline"
                onClick={() => {
                  setStatus('')
                  setSearch('')
                }}
              >
                {t('agentPage.runs.clearFilters')}
              </Button>
            ) : (
              <Button onClick={onChat}>{t('agentPage.chatWith')}</Button>
            )}
          </EmptyContent>
        </Empty>
      ) : (
        <>
          <ul
            aria-label={t('agentPage.tabs.runs')}
            className="flex flex-col divide-y"
          >
            {rows.map(run => (
              <AgentRunRow key={run.id} run={run} />
            ))}
          </ul>
          {rows.length < total && (
            <Button
              variant="ghost"
              className="self-center"
              onClick={() => setPages(current => current + 1)}
            >
              {t('agentPage.runs.loadMore', { shown: rows.length, total })}
            </Button>
          )}
        </>
      )}
    </div>
  )
}
