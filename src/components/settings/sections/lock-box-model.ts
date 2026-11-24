/**
 * Settings › Lock box: the pure rules the view follows. The core checks
 * every rule again when a key is saved and each time one is released
 * (`services::lock_box`); this only shapes the form and says early what
 * it will refuse.
 */
import type {
  KeyHeader,
  KeyKind,
  KeyRules,
  KeyView,
  LockLevel,
} from '@/lib/bindings'

export const KEY_KINDS: KeyKind[] = ['apiKey', 'token', 'password']
export const LOCK_LEVELS: LockLevel[] = ['open', 'ask', 'locked']
export const UNLOCK_MINUTES = [5, 15, 60, 240] as const
export const DEFAULT_UNLOCK_MINUTES = 15

/** How the proxy puts the key in a request */
export type HeaderPreset = 'bearer' | 'apiKey' | 'custom' | 'none'

export const HEADER_PRESETS: HeaderPreset[] = [
  'bearer',
  'apiKey',
  'custom',
  'none',
]

/** What the form edits */
export interface KeyForm {
  name: string
  kind: KeyKind
  /** One site per line or comma */
  hosts: string
  agents: string[]
  lock: LockLevel
  unlockMinutes: number
  header: HeaderPreset
  customHeader: string
  customPrefix: string
}

export function emptyForm(): KeyForm {
  return {
    name: '',
    kind: 'apiKey',
    hosts: '',
    agents: [],
    lock: 'ask',
    unlockMinutes: DEFAULT_UNLOCK_MINUTES,
    header: 'bearer',
    customHeader: '',
    customPrefix: '',
  }
}

/** The preset a saved header reads as */
export function headerPreset(header: KeyHeader | null | undefined): {
  preset: HeaderPreset
  name: string
  prefix: string
} {
  if (!header) return { preset: 'none', name: '', prefix: '' }
  const prefix = header.prefix ?? ''
  if (header.name === 'authorization' && prefix === 'Bearer ')
    return { preset: 'bearer', name: '', prefix: '' }
  if (header.name === 'x-api-key' && prefix === '')
    return { preset: 'apiKey', name: '', prefix: '' }
  return { preset: 'custom', name: header.name, prefix }
}

/** The form for a saved key */
export function formOf(key: KeyView): KeyForm {
  const header = headerPreset(key.rules.header)
  return {
    name: key.name,
    kind: key.rules.kind,
    hosts: key.rules.hosts.join('\n'),
    agents: [...key.rules.agents],
    lock: key.rules.lock ?? 'ask',
    unlockMinutes: key.rules.unlockMinutes ?? DEFAULT_UNLOCK_MINUTES,
    header: header.preset,
    customHeader: header.name,
    customPrefix: header.prefix,
  }
}

/** The sites typed, trimmed, one per line or comma */
export function parseHosts(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[\n,]/)
        .map(host => host.trim().toLowerCase())
        .filter(Boolean)
    ),
  ]
}

function headerOf(form: KeyForm): KeyHeader | null {
  switch (form.header) {
    case 'bearer':
      return { name: 'authorization', prefix: 'Bearer ' }
    case 'apiKey':
      return { name: 'x-api-key', prefix: '' }
    case 'custom':
      return {
        name: form.customHeader.trim().toLowerCase(),
        prefix: form.customPrefix,
      }
    case 'none':
      return null
  }
}

/** The rules the core gets */
export function rulesOf(form: KeyForm): KeyRules {
  return {
    kind: form.kind,
    hosts: parseHosts(form.hosts),
    agents: form.agents,
    lock: form.lock,
    unlockMinutes: form.unlockMinutes,
    header: headerOf(form),
  }
}

const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/
/** A bound site: a name, or `*.` and a name; no scheme, port or path */
const HOST = /^(\*\.)?([^\s:/@?#*.]+\.)+[^\s:/@?#*.]+$/

/** Whether `host` looks like a site the core will accept */
export function isBoundHost(host: string): boolean {
  return HOST.test(host)
}

/**
 * The i18n key of why the form can't be saved, or `null`. `value` is the
 * key's value, typed once (only when adding)
 */
export function formProblem(
  form: KeyForm,
  value: string | null
): string | null {
  if (!NAME.test(form.name.trim().toLowerCase())) return 'lockBox.problem.name'
  if (value !== null && !value.trim()) return 'lockBox.problem.value'
  if (value !== null && /[\r\n]/.test(value.trim()))
    return 'lockBox.problem.valueLines'
  if (!parseHosts(form.hosts).every(isBoundHost)) return 'lockBox.problem.host'
  if (form.header === 'custom' && !/^[a-z0-9_-]+$/i.test(form.customHeader))
    return 'lockBox.problem.header'
  return null
}

/** The i18n key of a key's lock level */
export function lockKey(lock: LockLevel | undefined): string {
  return `lockBox.lock.${lock ?? 'ask'}`
}

/** Whether a key can't be released at all, and why (an i18n key) */
export function keyWarning(key: KeyView): string | null {
  if (!key.stored) return 'lockBox.warning.missing'
  if (key.rules.hosts.length === 0) return 'lockBox.warning.noSites'
  if (key.rules.agents.length === 0) return 'lockBox.warning.noAgents'
  if (!key.rules.header) return 'lockBox.warning.noHeader'
  return null
}
