/**
 * Library card text: friendly names and the permissions line, from the
 * pure model's phrases (`library-model.ts`).
 */
import {
  BUILTIN_TOOL_LABELS,
  type LibraryItem,
  type PermissionPhrase,
} from './library-model'

type T = (key: string, options?: Record<string, unknown>) => string

/** `read_file` → `readFile`, the key of a built-in's friendly name */
export function toolLabel(name: string): string {
  return BUILTIN_TOOL_LABELS[name] ?? name
}

/** Built-ins get their friendly name; everything else its own */
export function displayName(item: LibraryItem, t: T): string {
  return item.type === 'builtinTool'
    ? t(`library.tool.${toolLabel(item.name)}`, { defaultValue: item.name })
    : item.name
}

/** One sentence of what it can do (§4.2); "No special access" when none */
export function permissionsLine(phrases: PermissionPhrase[], t: T): string {
  if (phrases.length === 0) return t('library.permission.none')
  return phrases
    .map(phrase => {
      switch (phrase.key) {
        case 'internet':
          return t('library.permission.internet', { count: phrase.count })
        case 'pilots':
        case 'aws':
          return t(`library.permission.${phrase.key}`, { names: phrase.names })
        default:
          return t(`library.permission.${phrase.key}`)
      }
    })
    .join(' · ')
}
