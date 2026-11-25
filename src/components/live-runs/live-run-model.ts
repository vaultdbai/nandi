/**
 * Ask's Live chats: every running or waiting run, however it started,
 * shown as a chat. Pure: the views pass in the runs, the agents and the
 * messages, and render what comes back.
 */
import type { TFunction } from 'i18next'
import type { RunPlan, RunStatus, RunSummary, SourceType } from '@/lib/bindings'
import type { UiMessage } from '@/types/agent'

/** A label to translate (`key`, with `values`), or a name as it is */
export type LiveLabel =
  { key: string; values?: Record<string, string> } | { text: string }

/** A label in words */
export function labelText(label: LiveLabel, t: TFunction): string {
  return 'text' in label ? label.text : t(label.key, label.values)
}

/** Whether a run is live work: going, or held for the person's approval */
export function isLiveStatus(status: RunStatus): boolean {
  return status === 'running' || status === 'awaitingApproval'
}

/** How long a finished run stays in the Live section (`LIVE_FINISHED_MS`) */
export const FINISHED_FOR_MS = 60 * 60 * 1000

/** Needs you (an approval waits), working, or finished a moment ago */
export type LiveGroup = 'needsYou' | 'working' | 'finished'

export function liveGroup(run: Pick<RunSummary, 'status'>): LiveGroup {
  if (run.status === 'awaitingApproval') return 'needsYou'
  return isLiveStatus(run.status) ? 'working' : 'finished'
}

const GROUP_ORDER: Record<LiveGroup, number> = {
  needsYou: 0,
  working: 1,
  finished: 2,
}

const ENDED = new Set<RunStatus>(['completed', 'failed', 'cancelled'])

/**
 * The Live section's rows: what needs the person first, then what's
 * working, then what finished in the last hour; newest first in each
 */
export function orderLive(
  runs: readonly RunSummary[],
  now: number
): RunSummary[] {
  const recent = (run: RunSummary) =>
    isLiveStatus(run.status) ||
    (ENDED.has(run.status) &&
      run.endedAt !== null &&
      now - run.endedAt <= FINISHED_FOR_MS)
  const latest = (run: RunSummary) => run.endedAt ?? run.startedAt
  return runs
    .filter(recent)
    .sort(
      (a, b) =>
        GROUP_ORDER[liveGroup(a)] - GROUP_ORDER[liveGroup(b)] ||
        latest(b) - latest(a)
    )
}

/** What it's doing now, in words: a plan step, or its latest message */
export interface StatusLine {
  text: string
  /** The plan step's number and the plan's size, when there's a plan */
  step?: { number: number; total: number }
}

/** "Step 2 of 5 · Reading 3 emails from Acme" */
export function statusText(line: StatusLine, t: TFunction): string {
  return line.step
    ? t('liveWork.stepLine', { ...line.step, text: line.text })
    : line.text
}

/** A status line stays one short line */
const MAX_LINE = 120

function firstLine(text: string): string {
  const line = text.trim().split('\n')[0]?.trim() ?? ''
  return line.length > MAX_LINE ? `${line.slice(0, MAX_LINE - 1)}…` : line
}

/**
 * What the run is doing now: the plan step it's on (or the next one),
 * else the agent's latest words. Never a tool's name; `null` when there's
 * nothing to say yet.
 */
export function statusLine(
  plan: RunPlan | null | undefined,
  messages: readonly UiMessage[]
): StatusLine | null {
  const steps = plan?.steps ?? []
  const index = [
    steps.findIndex(step => step.status === 'doing'),
    steps.findIndex(step => step.status === 'pending'),
  ].find(i => i !== -1)
  const step = index === undefined ? undefined : steps[index]
  if (step && index !== undefined) {
    return {
      text: step.title,
      step: { number: index + 1, total: steps.length },
    }
  }
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i]
    if (message?.kind === 'agent' && message.content.trim()) {
      return { text: firstLine(message.content) }
    }
  }
  return null
}

/** The trigger sources: their runs carry the trigger's name as the label */
const TRIGGER_SOURCES: ReadonlySet<SourceType> = new Set([
  'cron',
  'calendar',
  'file',
  'webhook',
  'device',
  'event',
])

/**
 * The local agent behind a requester: a card ID is `<instance>/<agent>`,
 * a local agent's is its own ID
 */
function requesterAgent(sourceId: string): string {
  return sourceId.slice(sourceId.lastIndexOf('/') + 1)
}

/**
 * How the run started, in a few words: "You asked", a trigger's name,
 * "Workflow: Pay bills", "Asked by Nandi"
 */
export function liveOrigin(
  run: Pick<RunSummary, 'sourceType' | 'sourceId' | 'sourceLabel'>,
  agentName: (agentId: string) => string | undefined
): LiveLabel {
  const label = run.sourceLabel?.trim() || null
  switch (run.sourceType) {
    case 'user':
      return { key: 'liveWork.origin.you' }
    case 'workflow':
      return label
        ? { key: 'liveWork.origin.workflow', values: { name: label } }
        : { key: 'liveWork.origin.aWorkflow' }
    case 'agent': {
      const asker = run.sourceId && agentName(requesterAgent(run.sourceId))
      if (asker)
        return { key: 'liveWork.origin.askedBy', values: { name: asker } }
      return label ? { text: label } : { key: 'liveWork.origin.anotherAgent' }
    }
    case 'email':
      return { key: 'liveWork.origin.email' }
    case 'calendarCheck':
      return { key: 'liveWork.origin.calendar' }
    default:
      if (label) return { text: label }
      return TRIGGER_SOURCES.has(run.sourceType)
        ? { key: 'liveWork.origin.trigger' }
        : { key: 'liveWork.origin.scheduled' }
  }
}

/**
 * The live chat's messages: the conversation from the run's first message
 * on, so later turns (the person's replies) follow it. Empty until the
 * run's first message is in.
 */
export function messagesFrom(
  messages: readonly UiMessage[],
  runId: string
): UiMessage[] {
  const start = messages.findIndex(message => message.runId === runId)
  return start === -1 ? [] : messages.slice(start)
}
