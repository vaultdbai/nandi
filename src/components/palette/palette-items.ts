import type { TFunction } from 'i18next'
import type { AppCommand } from '@/lib/commands/types'
import type { JsonValue, PaletteResult } from '@/lib/bindings'

/** Result kinds, in display order (`ask` sends the query to Studio) */
export const PALETTE_KINDS = [
  'ask',
  'action',
  'agent',
  'workflow',
  'run',
  'package',
  'tool',
] as const

export type PaletteKind = (typeof PALETTE_KINDS)[number]

/**
 * One row of the palette: a server result (`searchPalette`) or a local
 * app command from the command registry (sidebars, preferences, window).
 */
export interface PaletteItem {
  /** Unique within a result list */
  key: string
  kind: PaletteKind
  id: string
  name: string
  description: string
  score: number
  source: 'server' | 'local'
  args?: JsonValue | null
  /** A navigation-store `View` as JSON */
  navigate?: JsonValue | null
  shortcut?: string
}

export interface PaletteGroup {
  /** A `PaletteKind`, or `recent` */
  id: PaletteKind | 'recent'
  items: PaletteItem[]
}

const isKind = (kind: string): kind is PaletteKind =>
  (PALETTE_KINDS as readonly string[]).includes(kind)

const itemKey = (kind: string, id: string, args?: JsonValue | null) =>
  `${kind}:${id}:${args === undefined || args === null ? '' : JSON.stringify(args)}`

export function fromServer(result: PaletteResult): PaletteItem {
  const kind = isKind(result.kind) ? result.kind : 'action'
  return {
    key: itemKey(kind, result.id, result.args),
    kind,
    id: result.id,
    name: result.name,
    description: result.description,
    score: result.score,
    source: 'server',
    args: result.args ?? null,
    navigate: result.navigate ?? null,
  }
}

export function fromCommand(command: AppCommand, t: TFunction): PaletteItem {
  return {
    key: itemKey('local', command.id),
    kind: 'action',
    id: command.id,
    name: t(command.labelKey),
    description: command.descriptionKey ? t(command.descriptionKey) : '',
    score: 0,
    source: 'local',
    shortcut: command.shortcut,
  }
}

/**
 * The server's ranking, for local commands: exact > prefix > word prefix >
 * substring > description/keyword (see `services/palette.rs`).
 */
export function scoreText(
  query: string,
  name: string,
  description: string,
  keywords: string[] = []
): number {
  const q = query.trim().toLowerCase()
  if (!q) return 1
  const n = name.toLowerCase()
  if (n === q) return 100
  if (n.startsWith(q)) return 80
  const words = n.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
  const tokens = q.split(/\s+/)
  if (tokens.every(token => words.some(word => word.startsWith(token)))) {
    return 60
  }
  if (n.includes(q)) return 40
  const d = description.toLowerCase()
  const anywhere = (token: string) =>
    words.some(word => word.startsWith(token)) ||
    d.includes(token) ||
    keywords.some(k => k.toLowerCase().startsWith(token))
  return tokens.every(anywhere) ? 20 : 0
}

/** Groups by kind (in `PALETTE_KINDS` order), best first within each */
export function groupItems(items: PaletteItem[]): PaletteGroup[] {
  return PALETTE_KINDS.map(kind => ({
    id: kind,
    items: items
      .filter(item => item.kind === kind)
      .sort((a, b) => b.score - a.score),
  })).filter(group => group.items.length > 0)
}

/** The first row while typing: "Ask Nandi: <text>", sent to Studio */
export function askItem(text: string, t: TFunction): PaletteItem {
  return {
    key: 'ask:nandi',
    kind: 'ask',
    id: 'ask-nandi',
    name: t('palette.askNandi', { text }),
    description: t('palette.askNandiDescription'),
    score: 0,
    source: 'local',
    args: text,
  }
}

/** `>` at the start switches to command mode */
export function parseQuery(raw: string): {
  commandMode: boolean
  text: string
} {
  const trimmed = raw.trimStart()
  return trimmed.startsWith('>')
    ? { commandMode: true, text: trimmed.slice(1).trim() }
    : { commandMode: false, text: raw.trim() }
}
