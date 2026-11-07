import { formatRelativeTime } from '@/lib/format-time'
import type { IntakeStatus, IntakeThread, Request } from '@/lib/bindings'
import {
  queueRow,
  SOURCE_KEYS,
  type QueueRow,
} from '@/components/home/digest-model'

/** An item the user has already decided: one history line */
export interface DecidedRow {
  id: string
  status: Exclude<IntakeStatus, 'pending'>
  /** i18n key of "Approved" or "Dismissed" */
  statusKey: string
  text: string
  when: string
}

export interface ThreadModel {
  /** i18n key of where the thread came from */
  sourceKey: string
  /** Waiting for the user, oldest first: they get Approve and Dismiss */
  pending: QueueRow[]
  /** Already decided, oldest first */
  decided: DecidedRow[]
}

export function threadModel(
  thread: IntakeThread,
  locale: string,
  now: Date = new Date()
): ThreadModel {
  const pending: QueueRow[] = []
  const decided: DecidedRow[] = []
  for (const work of thread.items) {
    if (work.status === 'pending') {
      pending.push(queueRow(work, locale, now))
    } else {
      decided.push({
        id: work.id,
        status: work.status,
        statusKey: `intakeThread.status.${work.status}`,
        text: work.text,
        when: formatRelativeTime(work.at, locale, now),
      })
    }
  }
  return { sourceKey: SOURCE_KEYS[thread.source], pending, decided }
}

/**
 * Whether "Mark done" works on this Request's steps: only the user's own
 * local Requests have steps the user completes (a teammate's task does not)
 */
export function canMarkStepsDone(request: Request): boolean {
  return request.direction === 'local' && request.capability === ''
}
