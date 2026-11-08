/**
 * Pure rules for the agent page's Playbooks tab: the editor's draft, the
 * same checks the core makes (`domain::playbook`), so the form says what's
 * wrong before saving, and a name made from a title. No React, so all of
 * it is unit-tested.
 */
import type { Playbook, PlaybookInput } from '@/lib/bindings'

/** As in `domain::playbook` */
export const MAX_PLAYBOOK_WORDS = 900
export const MAX_NAME_CHARS = 64
export const MAX_LINE_CHARS = 200

export interface PlaybookDraft {
  name: string
  description: string
  when: string
  body: string
}

export const EMPTY_DRAFT: PlaybookDraft = {
  name: '',
  description: '',
  when: '',
  body: '',
}

export function draftOf(playbook: Playbook): PlaybookDraft {
  return {
    name: playbook.name,
    description: playbook.description,
    when: playbook.when,
    body: playbook.body,
  }
}

export function sameDraft(a: PlaybookDraft, b: PlaybookDraft): boolean {
  return (
    a.name.trim() === b.name.trim() &&
    a.description.trim() === b.description.trim() &&
    a.when.trim() === b.when.trim() &&
    a.body.trim() === b.body.trim()
  )
}

export function toInput(draft: PlaybookDraft): PlaybookInput {
  return {
    name: draft.name.trim(),
    description: draft.description.trim(),
    when: draft.when.trim(),
    body: draft.body.trim(),
  }
}

export function wordCount(text: string): number {
  const trimmed = text.trim()
  return trimmed ? trimmed.split(/\s+/).length : 0
}

const NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/

export function isValidName(name: string): boolean {
  return name.length <= MAX_NAME_CHARS && NAME.test(name)
}

/** A kebab-case name from what the user typed: "Pay a bill" → "pay-a-bill" */
export function playbookNameFrom(text: string): string {
  return text
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, MAX_NAME_CHARS)
    .replace(/-+$/, '')
}

export type PlaybookIssue =
  | 'nameRequired'
  | 'nameFormat'
  | 'nameTaken'
  | 'descriptionRequired'
  | 'whenRequired'
  | 'lineTooLong'
  | 'oneLine'
  | 'bodyRequired'
  | 'tooLong'

/**
 * What stops `draft` from saving. `taken`: the agent's playbook names;
 * `previous`: the name it had (`null` for a new one), which it may keep.
 */
export function draftIssues(
  draft: PlaybookDraft,
  taken: string[],
  previous: string | null
): PlaybookIssue[] {
  const issues: PlaybookIssue[] = []
  const name = draft.name.trim()
  if (!name) issues.push('nameRequired')
  else if (!isValidName(name)) issues.push('nameFormat')
  else if (name !== previous && taken.includes(name)) issues.push('nameTaken')
  const lines = [draft.description.trim(), draft.when.trim()]
  if (!lines[0]) issues.push('descriptionRequired')
  if (!lines[1]) issues.push('whenRequired')
  if (lines.some(line => line.includes('\n'))) issues.push('oneLine')
  if (lines.some(line => line.length > MAX_LINE_CHARS))
    issues.push('lineTooLong')
  if (!draft.body.trim()) issues.push('bodyRequired')
  else if (wordCount(draft.body) > MAX_PLAYBOOK_WORDS) issues.push('tooLong')
  return issues
}

/** Which playbook the editor shows: one by name, a new one, or none */
export type PlaybookSelection =
  { kind: 'none' } | { kind: 'new' } | { kind: 'existing'; name: string }

/** The selection after the list changed: a removed playbook isn't shown */
export function keepSelection(
  selection: PlaybookSelection,
  names: string[]
): PlaybookSelection {
  if (selection.kind === 'existing' && !names.includes(selection.name)) {
    return names[0] ? { kind: 'existing', name: names[0] } : { kind: 'none' }
  }
  if (selection.kind === 'none' && names[0]) {
    return { kind: 'existing', name: names[0] }
  }
  return selection
}
