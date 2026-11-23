/**
 * The weekly retro's Inbox items (`services::retro`): a proposed change,
 * a change applied this week (with its Undo), or a change the check says
 * didn't help (the note). Pure, so the card's wording and diff are tested
 * without rendering.
 */
import type { RetroItem, RetroRate } from '@/lib/bindings'
import type { DiffLine } from '@/components/agent-console/playbook-proposal-model'

/** What the card is about now */
export type RetroCardState = 'proposal' | 'applied' | 'didntHelp'

export function retroCardState(item: RetroItem): RetroCardState {
  if (item.status === 'pending') return 'proposal'
  return item.needsNote ? 'didntHelp' : 'applied'
}

/** Whether it waits on the person (a proposal), or is only offered */
export function isRetroWaiting(item: RetroItem): boolean {
  return item.status === 'pending'
}

/** What a proposal changes, as a translation key and its values */
export function retroTargetLabel(target: string): {
  key: string
  values: Record<string, string>
} {
  const [kind, ...rest] = target.split(':')
  const name = rest.join(':')
  switch (kind) {
    case 'playbook':
      return { key: 'retro.target.playbook', values: { name } }
    case 'workflow':
      return { key: 'retro.target.workflow', values: { name } }
    case 'trigger':
      return { key: 'retro.target.trigger', values: { name } }
    default:
      return { key: 'retro.target.skill', values: {} }
  }
}

/**
 * A unified diff's lines, without its headers: kept, removed and added
 * lines in reading order
 */
export function retroDiffLines(diff: string): DiffLine[] {
  const lines: DiffLine[] = []
  for (const line of diff.split('\n')) {
    if (
      line.startsWith('+++') ||
      line.startsWith('---') ||
      line.startsWith('@@') ||
      line.startsWith('diff ') ||
      line.startsWith('index ') ||
      line.startsWith('\\')
    ) {
      continue
    }
    if (line.startsWith('+')) lines.push({ kind: 'added', text: line.slice(1) })
    else if (line.startsWith('-'))
      lines.push({ kind: 'removed', text: line.slice(1) })
    else if (line.startsWith(' '))
      lines.push({ kind: 'same', text: line.slice(1) })
  }
  return lines
}

/** A correction rate as a whole percentage */
export function ratePercent(rate: RetroRate): number {
  if (rate.decisions === 0) return 0
  return Math.round((rate.corrections / rate.decisions) * 100)
}
