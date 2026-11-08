/**
 * A proposed playbook (`skill_propose`, `services/playbooks.rs`): what the
 * approval card shows before anything is written. Pure.
 *
 * Rust adds `current` (the playbook of that name it replaces) from the
 * agent's files, so a change is shown against what is really there.
 */

/** The agent tool that proposes a playbook */
export const PROPOSE_PLAYBOOK_TOOL = 'skill_propose'

export interface PlaybookFields {
  description: string
  when: string
  body: string
}

export interface PlaybookProposal extends PlaybookFields {
  name: string
  reason: string
  /** The playbook it replaces; `null` for a new one */
  current: PlaybookFields | null
}

export interface DiffLine {
  kind: 'same' | 'added' | 'removed'
  text: string
}

type Record_ = Record<string, unknown>

function text(record: Record_, key: string): string {
  const value = record[key]
  return typeof value === 'string' ? value.trim() : ''
}

function fields(record: Record_): PlaybookFields {
  return {
    description: text(record, 'description'),
    when: text(record, 'when'),
    body: text(record, 'body'),
  }
}

function isRecord(value: unknown): value is Record_ {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The proposal in a `skill_propose` approval, or `null` for another tool */
export function parsePlaybookProposal(
  tool: string,
  args: unknown
): PlaybookProposal | null {
  if (tool !== PROPOSE_PLAYBOOK_TOOL || !isRecord(args)) return null
  const name = text(args, 'name')
  if (!name) return null
  const current = isRecord(args.current) ? fields(args.current) : null
  return { name, reason: text(args, 'reason'), ...fields(args), current }
}

/** The one-line fields a change edits (`description`, `when`) */
export function changedLines(
  proposal: PlaybookProposal
): ('description' | 'when')[] {
  const { current } = proposal
  if (!current) return []
  return (['description', 'when'] as const).filter(
    key => current[key] !== proposal[key]
  )
}

/** Longest bodies diffed line by line; past that, old and new in full */
const MAX_DIFF_LINES = 400

/**
 * `before` → `after`, line by line (longest common subsequence): kept
 * lines, removed ones and added ones, in reading order
 */
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before === '' ? [] : before.split('\n')
  const b = after === '' ? [] : after.split('\n')
  if (a.length > MAX_DIFF_LINES || b.length > MAX_DIFF_LINES) {
    return [
      ...a.map(line => ({ kind: 'removed' as const, text: line })),
      ...b.map(line => ({ kind: 'added' as const, text: line })),
    ]
  }
  // common(i, j): the common length of a[i..] and b[j..], in a flat table
  const width = b.length + 1
  const table = new Array<number>((a.length + 1) * width).fill(0)
  const common = (i: number, j: number) => table[i * width + j] ?? 0
  const lineA = (i: number) => a[i] ?? ''
  const lineB = (j: number) => b[j] ?? ''
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i * width + j] =
        lineA(i) === lineB(j)
          ? common(i + 1, j + 1) + 1
          : Math.max(common(i + 1, j), common(i, j + 1))
    }
  }
  const out: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    if (lineA(i) === lineB(j)) {
      out.push({ kind: 'same', text: lineA(i) })
      i++
      j++
    } else if (common(i + 1, j) >= common(i, j + 1)) {
      out.push({ kind: 'removed', text: lineA(i) })
      i++
    } else {
      out.push({ kind: 'added', text: lineB(j) })
      j++
    }
  }
  for (; i < a.length; i++) out.push({ kind: 'removed', text: lineA(i) })
  for (; j < b.length; j++) out.push({ kind: 'added', text: lineB(j) })
  return out
}
