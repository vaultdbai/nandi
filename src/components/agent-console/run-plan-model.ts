/**
 * The run plan's checklist card, as data: which plan tool calls a message
 * list shows as the card (and which it hides), and how each step reads.
 * Pure; `RunPlanCard` renders it.
 */
import type {
  ChatMessage,
  RunPlan,
  RunPlanStep,
  RunStepStatus,
} from '@/lib/bindings'

export const PLAN_SET = 'plan_set'
export const PLAN_UPDATE = 'plan_update'

type ToolCallMessage = Extract<ChatMessage, { kind: 'toolCall' }>

const isToolCall = (message: ChatMessage): message is ToolCallMessage =>
  message.kind === 'toolCall'

/** A `plan_set` call shown as the run's checklist card */
export function isPlanCard(message: ChatMessage): message is ToolCallMessage {
  return (
    isToolCall(message) &&
    message.tool === PLAN_SET &&
    !message.isError &&
    message.runId !== null
  )
}

/**
 * The plan tool calls a message list leaves out: every successful
 * `plan_update` (the card shows its effect), and every `plan_set` but the
 * run's last, so each run shows one live card. Refused calls stay, as tool
 * cards, so the reason shows.
 */
export function hiddenPlanCalls(messages: readonly ChatMessage[]): Set<string> {
  const hidden = new Set<string>()
  const lastCard = new Map<string, string>()
  for (const message of messages) {
    if (!isToolCall(message) || message.isError) continue
    if (message.tool === PLAN_UPDATE) hidden.add(message.id)
    if (isPlanCard(message) && message.runId !== null) {
      const previous = lastCard.get(message.runId)
      if (previous) hidden.add(previous)
      lastCard.set(message.runId, message.id)
    }
  }
  return hidden
}

/** "3 of 7": steps closed (done or skipped) out of all */
export function planProgress(plan: RunPlan): { closed: number; total: number } {
  return {
    closed: plan.steps.filter(
      step => step.status === 'done' || step.status === 'skipped'
    ).length,
    total: plan.steps.length,
  }
}

/** Each status's colour, from the theme tokens */
export const STEP_STATUS_CLASSES: Record<RunStepStatus, string> = {
  pending: 'text-muted-foreground',
  doing: 'text-info',
  done: 'text-success',
  skipped: 'text-muted-foreground',
  failed: 'text-destructive',
}

const IMAGE = /\.(png|jpe?g)$/i

/** An evidence file the card shows as a thumbnail */
export function isScreenshot(path: string): boolean {
  return IMAGE.test(path)
}

/** `browser/r1/3-1.png` → `3-1.png` */
export function fileName(path: string): string {
  return path.split('/').at(-1) ?? path
}

/** Marked `screenshot`, done, and nothing to show for it */
export function missesScreenshot(step: RunPlanStep): boolean {
  return (
    step.evidence === 'screenshot' &&
    step.status === 'done' &&
    step.files.length === 0
  )
}
