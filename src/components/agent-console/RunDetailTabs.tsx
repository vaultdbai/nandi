/**
 * Tab panels of the run detail view (Outputs, Files, Logs, Analysis,
 * Approvals) and the "Run details" metadata list. Each takes the loaded
 * `RunDetail`.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  Check,
  ChevronRight,
  Copy,
  ExternalLink,
  FileText,
  FolderOpen,
  Info,
  ShieldCheck,
  X,
} from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { displayAgent } from '@/lib/agent-identity'
import { client, isDesktop } from '@/lib/client'
import { modelLabel } from '@/lib/models'
import { openSettings } from '@/lib/settings-nav'
import type { RunDetail, RunSummary } from '@/lib/tauri-bindings'
import { decideApproval } from '@/lib/agent-actions'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { ApprovalRequestCard } from './ApprovalRequestCard'
import { AttachmentChip } from './AttachmentChip'
import { MarkdownContent } from './MarkdownContent'
import { MessageList } from './MessageList'
import { formatJson } from './format-json'
import {
  filesTouched,
  finalOutput,
  formatBytes,
  type FileOp,
} from './run-detail-model'
import {
  filterLogs,
  LOG_KINDS,
  logEntry,
  ranOnCliRuntime,
  runtimeDecisions,
  type LogEntry,
  type LogKind,
} from './run-logs-model'

function EmptyTab({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: typeof FileText
  title: string
  description?: string
  /** The one next step */
  action?: React.ReactNode
}) {
  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Icon />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        {description && <EmptyDescription>{description}</EmptyDescription>}
      </EmptyHeader>
      {action && <EmptyContent>{action}</EmptyContent>}
    </Empty>
  )
}

// ---------------------------------------------------------------------------
// Outputs

export function RunOutputs({ detail }: { detail: RunDetail }) {
  const { t } = useTranslation()
  const output = finalOutput(detail)
  const attachments = detail.messages.flatMap(m =>
    m.kind === 'user' ? m.attachments : []
  )
  const agent = displayAgent(detail.run.agentId, detail.agentName)

  return (
    <div className="flex flex-col gap-4">
      <section className="rounded-lg border p-4">
        <h3 className="mb-2 text-sm font-semibold">
          {t('runDetail.outputs.final')}
        </h3>
        {output ? (
          <div className="text-sm">
            <MarkdownContent content={output} />
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t('runDetail.overview.noOutput')}
          </p>
        )}
      </section>

      {attachments.length > 0 && (
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">
            {t('runDetail.outputs.attachments')}
          </h3>
          <div className="flex flex-wrap gap-1.5">
            {attachments.map(attachment => (
              <AttachmentChip key={attachment.id} attachment={attachment} />
            ))}
          </div>
        </section>
      )}

      {detail.messages.length > 0 && (
        <Collapsible className="rounded-lg border">
          <CollapsibleTrigger className="group flex w-full items-center gap-2 px-3 py-2 text-sm font-medium">
            <ChevronRight className="size-4 transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90" />
            {t('runDetail.outputs.conversation', {
              count: detail.messages.length,
            })}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <div className="flex h-[32rem] flex-col border-t">
              <MessageList
                messages={detail.messages}
                assistant={agent}
                onDecide={(approvalId, decision) =>
                  decideApproval(detail.run.agentId, approvalId, decision)
                }
              />
            </div>
          </CollapsibleContent>
        </Collapsible>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Files

export function RunFiles({ detail }: { detail: RunDetail }) {
  const { t, i18n } = useTranslation()
  const files = filesTouched(detail)
  if (files.length === 0) {
    return (
      <EmptyTab
        icon={FileText}
        title={t('runDetail.files.emptyTitle')}
        description={t('runDetail.files.emptyDescription')}
      />
    )
  }
  return (
    <div className="overflow-x-auto rounded-lg border">
      <table className="w-full text-sm">
        <thead className="bg-muted/40 text-xs text-muted-foreground">
          <tr className="[&_th]:px-3 [&_th]:py-2 [&_th]:text-start [&_th]:font-medium">
            <th scope="col">{t('runDetail.files.operation')}</th>
            <th scope="col">{t('runDetail.files.path')}</th>
            <th scope="col">{t('runDetail.files.size')}</th>
            <th scope="col">
              <span className="sr-only">{t('runDetail.files.actions')}</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {files.map(file => (
            <tr key={file.key} className="border-t [&_td]:px-3 [&_td]:py-2">
              <td className="whitespace-nowrap">
                <Badge
                  variant="outline"
                  className={cn(file.isError && 'text-destructive')}
                >
                  {t(`runDetail.files.op.${file.op}`)}
                </Badge>
              </td>
              <td className="max-w-96">
                <code
                  dir="ltr"
                  className="block truncate font-mono text-xs"
                  title={file.path}
                >
                  {file.path}
                </code>
              </td>
              <td className="text-xs whitespace-nowrap text-muted-foreground tabular-nums">
                {file.sizeBytes === null
                  ? '—'
                  : formatBytes(file.sizeBytes, i18n.language)}
              </td>
              <td className="whitespace-nowrap text-end">
                {HAS_FILE_PATH.includes(file.op) && (
                  <FileActions runId={detail.run.id} path={file.path} />
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** Operations whose path names a file (a shell row holds a command) */
const HAS_FILE_PATH: readonly FileOp[] = ['read', 'write', 'list']

/**
 * Open, Reveal (desktop) and Copy path for one file the run touched. The
 * host path is resolved on demand; a file that's gone says so.
 */
function FileActions({ runId, path }: { runId: string; path: string }) {
  const { t } = useTranslation()
  const desktop = isDesktop()

  const open = async (reveal: boolean) => {
    const result = await client.openRunFile(runId, path, reveal)
    if (result.status === 'error') toast.error(result.error)
  }

  const copyPath = async () => {
    const resolved = await client.resolveRunFile(runId, path)
    const text =
      resolved.status === 'ok' && resolved.data !== null ? resolved.data : path
    await navigator.clipboard.writeText(text).then(
      () => toast.success(t('runDetail.files.pathCopied')),
      () => toast.error(t('runDetail.files.copyFailed'))
    )
  }

  return (
    <div className="flex justify-end gap-1">
      {desktop && (
        <>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t('runDetail.files.open', { path })}
            title={t('runDetail.files.openShort')}
            onClick={() => void open(false)}
          >
            <ExternalLink />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t('runDetail.files.reveal', { path })}
            title={t('runDetail.files.revealShort')}
            onClick={() => void open(true)}
          >
            <FolderOpen />
          </Button>
        </>
      )}
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label={t('runDetail.files.copyPath', { path })}
        title={t('runDetail.files.copyPathShort')}
        onClick={() => void copyPath()}
      >
        <Copy />
      </Button>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Logs

export function RunLogs({
  detail,
  highlightEventId,
}: {
  detail: RunDetail
  /** A cited event to scroll to */
  highlightEventId?: string | null
}) {
  const { t, i18n } = useTranslation()
  const [copied, setCopied] = useState(false)
  const [kind, setKind] = useState<LogKind | null>(null)
  const [query, setQuery] = useState('')
  const log = JSON.stringify(detail.events, null, 2)
  const entries = detail.events.map(logEntry)
  const lineOf = (entry: LogEntry) =>
    t(`runDetail.logs.line.${entry.line}`, entry.values)
  const shown = filterLogs(entries, { kind, text: query }, lineOf)

  const copy = () => {
    void navigator.clipboard.writeText(log).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      },
      () => toast.error(t('runDetail.logs.copyFailed'))
    )
  }

  if (detail.events.length === 0) {
    return (
      <EmptyTab
        icon={FileText}
        title={t('runDetail.logs.empty')}
        description={t('runDetail.logs.emptyDescription')}
      />
    )
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <div
          role="group"
          aria-label={t('runDetail.logs.filterKind')}
          className="flex flex-wrap gap-1"
        >
          {([null, ...LOG_KINDS] as const).map(value => (
            <Button
              key={value ?? 'all'}
              size="sm"
              variant={kind === value ? 'secondary' : 'ghost'}
              aria-pressed={kind === value}
              className="h-7 px-2 text-xs"
              onClick={() => setKind(value)}
            >
              {t(`runDetail.logs.kind.${value ?? 'all'}`)}
            </Button>
          ))}
        </div>
        <Input
          value={query}
          onChange={event => setQuery(event.target.value)}
          placeholder={t('runDetail.logs.search')}
          aria-label={t('runDetail.logs.search')}
          className="h-7 w-48 text-xs"
        />
        <span className="ms-auto text-xs text-muted-foreground">
          {t('runDetail.logs.count', { count: shown.length })}
        </span>
        <Button size="sm" variant="outline" onClick={copy}>
          {copied ? <Check /> : <Copy />}
          {copied ? t('runDetail.logs.copied') : t('runDetail.logs.copy')}
        </Button>
      </div>
      {shown.length === 0 ? (
        <p className="py-4 text-sm text-muted-foreground">
          {t('runDetail.logs.noMatch')}{' '}
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0"
            onClick={() => {
              setKind(null)
              setQuery('')
            }}
          >
            {t('runDetail.logs.clearFilters')}
          </Button>
        </p>
      ) : (
        <ol
          aria-label={t('runDetail.tabs.logs')}
          className="flex flex-col divide-y rounded-lg border text-xs"
        >
          {shown.map(entry => (
            <LogRow
              key={entry.id}
              entry={entry}
              line={lineOf(entry)}
              highlighted={highlightEventId === entry.id}
              time={new Date(entry.at).toLocaleTimeString(i18n.language, {
                hour: '2-digit',
                minute: '2-digit',
                second: '2-digit',
              })}
            />
          ))}
        </ol>
      )}
    </div>
  )
}

const KIND_CLASSES: Record<LogKind, string> = {
  model: 'text-info',
  tool: 'text-foreground',
  approval: 'text-warning',
  error: 'text-destructive',
  other: 'text-muted-foreground',
}

/** One event: time, kind, a readable line; its JSON on demand */
function LogRow({
  entry,
  line,
  time,
  highlighted,
}: {
  entry: LogEntry
  line: string
  time: string
  highlighted: boolean
}) {
  const { t } = useTranslation()
  return (
    <li
      id={`run-event-${entry.id}`}
      className={cn(highlighted && 'bg-primary/10')}
    >
      <Collapsible defaultOpen={highlighted}>
        <CollapsibleTrigger className="group flex w-full items-start gap-2 px-3 py-2 text-start hover:bg-accent/40">
          <ChevronRight
            aria-hidden
            className="mt-0.5 size-3.5 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90"
          />
          <time className="w-16 shrink-0 text-muted-foreground tabular-nums">
            {time}
          </time>
          <Badge
            variant="outline"
            className={cn('shrink-0 px-1.5 py-0', KIND_CLASSES[entry.kind])}
          >
            {t(`runDetail.logs.kind.${entry.kind}`)}
          </Badge>
          <span className="min-w-0 flex-1 break-words">{line}</span>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="px-3 pb-2 ps-9">
            <p className="mb-1 font-mono text-muted-foreground" dir="ltr">
              #{entry.id} · {entry.raw.kind}
            </p>
            <pre
              dir="ltr"
              className="overflow-x-auto rounded bg-muted/40 p-2 font-mono break-all whitespace-pre-wrap text-muted-foreground"
            >
              {formatJson(entry.raw.payload)}
            </pre>
          </div>
        </CollapsibleContent>
      </Collapsible>
    </li>
  )
}

// ---------------------------------------------------------------------------
// Analysis

// ---------------------------------------------------------------------------
// Approvals

/**
 * Every approval the run asked for. Pending ones can be decided here, the
 * same way as in chat and the Inbox.
 */
export function RunApprovals({ detail }: { detail: RunDetail }) {
  const { t, i18n } = useTranslation()
  const approvals = detail.messages.filter(m => m.kind === 'approval')
  // A CLI runtime answers the calls nobody had to approve on its own
  if (ranOnCliRuntime(detail) && approvals.length === 0) {
    return <RuntimeDecisions detail={detail} />
  }
  if (approvals.length === 0) {
    return (
      <EmptyTab
        icon={ShieldCheck}
        title={t('runDetail.approvals.emptyTitle')}
        description={t('runDetail.approvals.emptyDescription')}
      />
    )
  }
  return (
    <ul className="flex flex-col gap-2">
      {approvals.map(approval => (
        <li key={approval.id} className="flex flex-col gap-1">
          <ApprovalRequestCard
            message={approval}
            onDecide={(approvalId, decision) =>
              decideApproval(detail.run.agentId, approvalId, decision)
            }
          />
          <time
            dateTime={new Date(approval.createdAt).toISOString()}
            className="ps-1 text-xs text-muted-foreground"
          >
            {t('runDetail.approvals.requestedAt', {
              time: new Date(approval.createdAt).toLocaleString(i18n.language),
            })}
          </time>
        </li>
      ))}
    </ul>
  )
}

/**
 * A CLI runtime (Claude Code, Codex, Gemini CLI) run that asked nothing:
 * Nandi answered its tool requests from the agent's allowed tools, so list
 * what was allowed or denied
 */
function RuntimeDecisions({ detail }: { detail: RunDetail }) {
  const { t, i18n } = useTranslation()
  const decisions = runtimeDecisions(detail)
  return (
    <div className="flex flex-col gap-3">
      <div
        role="note"
        className="flex flex-wrap items-start gap-2 rounded-lg border border-info/30 bg-info/5 p-3 text-sm"
      >
        <Info aria-hidden className="mt-0.5 size-4 shrink-0 text-info" />
        <p className="min-w-0 flex-1">
          {t('runDetail.approvals.cliDecisions')}
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore.getState().navigate({
              kind: 'agent',
              agentId: detail.run.agentId,
              tab: 'settings',
            })
          }
        >
          {t('runDetail.approvals.changeSettings')}
        </Button>
      </div>
      {decisions.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('runDetail.approvals.noDecisions')}
        </p>
      ) : (
        <ul
          aria-label={t('runDetail.approvals.decisionsLabel')}
          className="flex flex-col divide-y rounded-lg border text-sm"
        >
          {decisions.map(decision => (
            <li key={decision.id} className="flex items-center gap-3 px-3 py-2">
              {decision.allowed ? (
                <Check aria-hidden className="size-4 shrink-0 text-success" />
              ) : (
                <X aria-hidden className="size-4 shrink-0 text-destructive" />
              )}
              <span className="shrink-0 font-medium">
                {t(
                  decision.allowed
                    ? 'runDetail.approvals.allowedAuto'
                    : 'runDetail.approvals.deniedAuto'
                )}
              </span>
              <span className="min-w-0 flex-1 truncate" title={decision.title}>
                {decision.title}
              </span>
              <time className="shrink-0 text-xs text-muted-foreground tabular-nums">
                {new Date(decision.at).toLocaleTimeString(i18n.language)}
              </time>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Metadata

/** Runs that ended (or paused) after planning steps without a model call */
const CANNED_STATUSES: readonly RunSummary['status'][] = [
  'completed',
  'awaitingApproval',
]

/**
 * The model the run called (every LLM call records it on the run). With
 * none, a run that still planned steps ran on built-in replies, which
 * means no model is set up (offer to add one); one that failed, was
 * cancelled, or hasn't started never reached a model.
 */
function RunEngine({ run }: { run: RunSummary }) {
  const { t } = useTranslation()
  if (run.model) {
    return (
      <code dir="ltr" className="font-mono text-xs" title={run.model}>
        {modelLabel(run.model)}
      </code>
    )
  }
  if (!CANNED_STATUSES.includes(run.status)) {
    return t('runs.meta.engineNoModel')
  }
  return (
    <span className="flex flex-wrap items-center gap-x-2 whitespace-normal">
      {t('runs.meta.engineCanned')}
      <Button
        variant="link"
        size="sm"
        className="h-auto p-0 text-xs"
        onClick={() => openSettings('models')}
      >
        {t('runs.meta.addModel')}
      </Button>
    </span>
  )
}

/** "You", or the workflow / trigger / delegating agent that asked */
function runSource(
  t: ReturnType<typeof useTranslation>['t'],
  run: RunSummary
): string {
  if (run.sourceType === 'user') return t('agentConsole.queue.source.user')
  return run.sourceLabel ?? t(`agentConsole.queue.source.${run.sourceType}`)
}

export function RunMetadata({
  detail,
  duration,
}: {
  detail: RunDetail
  duration: string
}) {
  const { t } = useTranslation()
  const { run } = detail
  const toolCalls = detail.messages.filter(m => m.kind === 'toolCall').length
  const rows: [string, React.ReactNode][] = [
    [t('runs.meta.engine'), <RunEngine key="engine" run={run} />],
    [t('runs.meta.source'), runSource(t, run)],
    [t('runs.meta.duration'), duration],
    [t('runs.meta.toolCalls'), toolCalls],
    [t('runs.meta.events'), detail.events.length],
  ]

  return (
    <aside
      aria-label={t('runs.meta.title')}
      className="flex flex-col gap-3 rounded-lg border p-3"
    >
      <h3 className="text-sm font-semibold">{t('runs.meta.title')}</h3>
      <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3 xl:grid-cols-5">
        {rows.map(([label, value]) => (
          <div key={label} className="flex min-w-0 flex-col gap-0.5">
            <dt className="text-xs text-muted-foreground">{label}</dt>
            <dd className="truncate text-foreground">{value}</dd>
          </div>
        ))}
      </dl>
    </aside>
  )
}
