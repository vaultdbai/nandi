/**
 * Live UI view model: how a file change reads ("Updated just now by Chief
 * of Staff") and where it leads. Pure, so the wording is tested once.
 */
import type { TFunction } from 'i18next'
import type { ChangeAuthor, LiveChange } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import type { View } from '@/store/navigation-store'

/** Under this, a change reads "just now" */
const JUST_NOW_MS = 60_000

/** "by you", "by Chief of Staff", "by the Weekly report workflow", "outside Nandi" */
export function authorText(author: ChangeAuthor, t: TFunction): string {
  switch (author.kind) {
    case 'you':
      return t('live.by.you')
    case 'agent':
      return t('live.by.agent', { name: author.agentName })
    case 'workflow':
      return t('live.by.workflow', { name: author.workflowName })
    case 'outside':
      return t('live.by.outside')
  }
}

/** "just now", else "5 min ago" */
export function whenText(
  at: number,
  now: number,
  t: TFunction,
  locale: string
): string {
  return now - at < JUST_NOW_MS
    ? t('live.justNow')
    : formatRelativeTime(at, locale, new Date(now))
}

/** "Updated just now by you" */
export function updatedText(
  change: LiveChange,
  now: number,
  t: TFunction,
  locale: string
): string {
  return t('live.updated', {
    when: whenText(change.at, now, t, locale),
    by: authorText(change.author, t),
  })
}

/** What a change was about: its name, else the area ("My notes") */
export function changeTitle(change: LiveChange, t: TFunction): string {
  return change.label || t(`live.area.${change.area}`)
}

/** "report.md, chart.png and 3 more" for a work folder; nothing for one file */
export function changeFiles(change: LiveChange, t: TFunction): string | null {
  if (change.area !== 'work' || change.paths.length === 0) return null
  const more = change.pathCount - change.paths.length
  const named = change.paths.join(', ')
  return more > 0 ? t('live.moreFiles', { files: named, count: more }) : named
}

/** The page that shows what changed */
export function viewOf(change: LiveChange): View | null {
  const id = change.targetId
  switch (change.area) {
    case 'agent':
    case 'work':
      return id ? { kind: 'agent', agentId: id } : { kind: 'agents' }
    case 'workflow':
      return id ? { kind: 'workflow', workflowId: id } : null
    case 'trigger':
      return id ? { kind: 'trigger', triggerId: id } : null
    case 'notes':
      return id && change.action !== 'deleted'
        ? { kind: 'notes', note: id }
        : { kind: 'notes' }
    case 'packs':
      return { kind: 'library' }
    case 'connectors':
      return { kind: 'library' }
    case 'settings':
    case 'profile':
      return { kind: 'settings' }
    case 'policy':
    case 'manifest':
      return null
  }
}
