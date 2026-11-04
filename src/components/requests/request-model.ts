/**
 * Pure rules for the Request page: which buttons a blocker offers, which
 * of them ask first (README principle 7), and how steps and banners look.
 * The copy itself is in `locales/*.json` under `request.*`.
 */
import type {
  Blocker,
  BlockerKind,
  JsonValue,
  Request,
  RequestStep,
  StepStatus,
} from '@/lib/bindings'

/** The keys an ask's own words travel under (as the core's `title_of`) */
const TEXT_KEYS = [
  'text',
  'request',
  'natural_language_request',
  'prompt',
  'query',
  'message',
  'input',
]

export interface Ask {
  /** The words asked, as written (`null`: only structured inputs) */
  text: string | null
  /** Every other input, as sent: due dates, links, parameters */
  fields: { key: string; value: string }[]
}

/** What was asked, word for word: its text and every other input */
export function askOf(input: JsonValue | undefined): Ask {
  if (typeof input === 'string') return { text: input, fields: [] }
  if (input === null || input === undefined) return { text: null, fields: [] }
  if (typeof input !== 'object' || Array.isArray(input)) {
    return { text: null, fields: [{ key: '', value: JSON.stringify(input) }] }
  }
  const textKey = TEXT_KEYS.find(key => typeof input[key] === 'string')
  return {
    text: textKey ? (input[textKey] as string) : null,
    fields: Object.entries(input)
      .filter(([key, value]) => key !== textKey && value !== undefined)
      .map(([key, value]) => ({
        key,
        value: inputValue(value as JsonValue),
      })),
  }
}

/** One input in words: links as their names and addresses */
function inputValue(value: JsonValue): string {
  if (typeof value === 'string') return value
  if (Array.isArray(value) && value.length > 0 && value.every(isLink)) {
    return value
      .map(link => (link.name ? `${link.name} (${link.url})` : link.url))
      .join(', ')
  }
  return JSON.stringify(value)
}

function isLink(value: JsonValue): value is { url: string; name?: string } {
  return (
    value !== null &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    typeof value.url === 'string'
  )
}

export type BannerAction =
  | 'approve'
  | 'reject'
  | 'answer'
  | 'trust'
  | 'block'
  | 'askTrust'
  | 'turnOn'
  | 'askTurnOn'
  | 'resume'
  | 'sendElsewhere'
  | 'cancel'
  | 'addModel'
  | 'addKey'
  | 'retry'
  | 'details'
  | 'remind'

export interface BannerActions {
  /** The one filled button (none: the banner only explains) */
  primary: BannerAction | null
  secondary: BannerAction[]
}

/** The fix buttons for a blocker (request-detail.md §5.2) */
export function bannerActions(
  request: Request,
  blocker: Blocker
): BannerActions {
  const outgoing = request.direction === 'outgoing'
  const of = (
    primary: BannerAction | null,
    ...secondary: BannerAction[]
  ): BannerActions => ({ primary, secondary })
  const actions: Record<BlockerKind, () => BannerActions> = {
    // A tool approval, or another Nandi's request held for the owner
    needsMyApproval: () =>
      blocker.approvalId || isGatewayHold(request, blocker)
        ? of('approve', 'reject')
        : of('details'),
    needsTheirApproval: () =>
      of('remind', ...(blocker.terminal ? [] : (['cancel'] as const))),
    needsMyAnswer: () => of('answer'),
    needsTheirAnswer: () => of(null, 'cancel'),
    notTrusted: () => (outgoing ? of('askTrust') : of('trust', 'block')),
    notAccepting: () =>
      outgoing ? of('askTurnOn') : of(blocker.agentId ? 'turnOn' : 'details'),
    pausedOrBlocked: () => (outgoing ? of('details') : of('resume')),
    queued: () => of(null, 'cancel'),
    offline: () => of('sendElsewhere', 'cancel'),
    noModel: () => of('addModel'),
    runtimeAuth: () => of('addKey'),
    timedOut: () => (outgoing ? of('retry', 'details') : of('details')),
    failed: () => (outgoing ? of('retry', 'details') : of('details')),
    // Their owner said no: try someone else; on my side, nothing to fix
    declined: () => (outgoing ? of('sendElsewhere', 'details') : of('details')),
  }
  return actions[blocker.kind]()
}

/**
 * Another Nandi's request this Nandi holds for my approval (task 5): Approve
 * and Reject decide the hold, not a tool approval
 */
export function isGatewayHold(request: Request, blocker: Blocker): boolean {
  return (
    request.direction === 'incoming' &&
    blocker.kind === 'needsMyApproval' &&
    blocker.stepId === 'approval'
  )
}

/** The banner's sentence key: a held request reads as one */
export function bannerSentenceKey(request: Request, blocker: Blocker): string {
  const side = request.direction === 'outgoing' ? 'outgoing' : 'incoming'
  if (isGatewayHold(request, blocker)) return 'request.blocker.hold.incoming'
  return `request.blocker.${blocker.kind}.${side}`
}

/** Actions that change something for someone else, or can't be undone */
const CONFIRMED: ReadonlySet<BannerAction> = new Set<BannerAction>([
  'reject',
  'trust',
  'block',
  'cancel',
])

export function needsConfirmation(action: BannerAction): boolean {
  return CONFIRMED.has(action)
}

export type BannerTone = 'warning' | 'neutral' | 'destructive'

/** Warning when it waits on me, destructive once it ended, else neutral */
export function bannerTone(blocker: Blocker): BannerTone {
  if (blocker.terminal) return 'destructive'
  return blocker.who === 'me' ? 'warning' : 'neutral'
}

/** Whether the header offers Stop */
/** A local Request's step position (0-based) from its ID (`step-<n>`) */
export function localStepPosition(step: RequestStep): number | null {
  const match = /^step-(\d+)$/.exec(step.id)
  return match ? Number(match[1]) - 1 : null
}

export function canStop(request: Request): boolean {
  return request.status === 'running' || request.status === 'waiting'
}

/** Plan steps, numbered from 1 (system steps get none) */
export function stepNumbers(steps: RequestStep[]): Map<string, number> {
  const numbers = new Map<string, number>()
  let n = 0
  for (const step of steps) {
    if (step.kind !== 'system') numbers.set(step.id, ++n)
  }
  return numbers
}

/** A rail marker per step status */
export const STEP_MARKERS: Record<StepStatus, string> = {
  done: '✓',
  running: '●',
  waiting: '⏸',
  failed: '✕',
  pending: '○',
  skipped: '⤼',
}

/** How long, from start to end (or now while it runs) */
export function stepDuration(
  step: Pick<RequestStep, 'startedAt' | 'endedAt'>,
  now: number
): number | null {
  if (step.startedAt === null) return null
  return Math.max(0, (step.endedAt ?? now) - step.startedAt)
}

/** The step the page opens on: the one the banner names, if any */
export function focusStep(request: Request, stepId?: string): string | null {
  return stepId ?? request.stuck?.stepId ?? null
}
