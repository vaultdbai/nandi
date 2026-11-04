/**
 * Reads and writes simple fields in an artifact's YAML (a Markdown file's
 * frontmatter, or a whole YAML file) line by line, so a form edit changes
 * only that line and keeps the user's formatting and comments. Paths are
 * `key` or `parent.key` (one level of nesting, two-space indent). Values
 * are scalars or flat lists, written inline (`[a, b]`).
 */

export type FieldValue = string | number | boolean | null | string[]

interface Region {
  lines: string[]
  /** First and one-past-last line of the YAML */
  start: number
  end: number
  eol: string
}

function region(text: string): Region {
  const eol = text.includes('\r\n') ? '\r\n' : '\n'
  const lines = text.split(eol)
  if (lines[0]?.trim() === '---') {
    const close = lines.findIndex((line, i) => i > 0 && line.trim() === '---')
    if (close !== -1) return { lines, start: 1, end: close, eol }
  }
  return { lines, start: 0, end: lines.length, eol }
}

const KEY = /^(\s*)([A-Za-z0-9_-]+):(.*)$/

/** Index of `key` at `indent` within [from, to), or -1 */
function find(
  lines: string[],
  from: number,
  to: number,
  key: string,
  indent: number
): number {
  for (let i = from; i < to; i++) {
    const match = KEY.exec(lines[i] ?? '')
    if (match?.[1]?.length === indent && match[2] === key) return i
  }
  return -1
}

/** Where a top-level key's block ends (next top-level line) */
function blockEnd(lines: string[], at: number, to: number): number {
  let i = at + 1
  while (i < to && (/^\s/.test(lines[i] ?? '') || lines[i]?.trim() === '')) i++
  return i
}

function parseScalar(raw: string): FieldValue {
  const value = raw.replace(/\s+#.*$/, '').trim()
  if (value === '' || value === 'null' || value === '~') return null
  if (value === 'true' || value === 'false') return value === 'true'
  if (/^-?\d+(\.\d+)?$/.test(value)) return Number(value)
  if (value.startsWith('[') && value.endsWith(']')) {
    return value
      .slice(1, -1)
      .split(',')
      .map(item => unquote(item.trim()))
      .filter(Boolean)
  }
  return unquote(value)
}

function unquote(value: string): string {
  if (
    value.length >= 2 &&
    ((value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'")))
  ) {
    return value.slice(1, -1)
  }
  return value
}

function format(value: FieldValue): string {
  if (value === null) return 'null'
  if (typeof value === 'boolean' || typeof value === 'number') {
    return String(value)
  }
  if (Array.isArray(value)) return `[${value.map(quote).join(', ')}]`
  return quote(value)
}

function quote(value: string): string {
  const plain =
    value !== '' &&
    !/^[\s\-?:,[\]{}#&*!|>'"%@`]/.test(value) &&
    !/[:#]\s|:$|\s$/.test(value) &&
    !/^(true|false|null|~|-?\d+(\.\d+)?)$/.test(value)
  return plain ? value : JSON.stringify(value)
}

/** The value at `path`, or `undefined` when it isn't there */
export function getField(text: string, path: string): FieldValue | undefined {
  const { lines, start, end } = region(text)
  const [first, second] = path.split('.')
  if (!first) return undefined
  const at = find(lines, start, end, first, 0)
  if (at === -1) return undefined
  if (second === undefined) {
    const rest = KEY.exec(lines[at] ?? '')?.[3] ?? ''
    if (rest.trim() !== '') return parseScalar(rest)
    // A block list under the key
    const items: string[] = []
    for (let i = at + 1; i < blockEnd(lines, at, end); i++) {
      const item = /^\s*-\s+(.*)$/.exec(lines[i] ?? '')
      if (item?.[1] !== undefined) items.push(unquote(item[1].trim()))
    }
    return items.length > 0 ? items : null
  }
  const child = find(lines, at + 1, blockEnd(lines, at, end), second, 2)
  if (child === -1) return undefined
  return parseScalar(KEY.exec(lines[child] ?? '')?.[3] ?? '')
}

/** `text` with `path` set to `value`, adding the key if it's missing */
export function setField(
  text: string,
  path: string,
  value: FieldValue
): string {
  const { lines, start, end, eol } = region(text)
  const [first, second] = path.split('.')
  if (!first) return text
  const at = find(lines, start, end, first, 0)
  if (second === undefined) {
    const line = `${first}: ${format(value)}`
    if (at === -1) {
      lines.splice(end, 0, line)
    } else {
      // Replace the key and any block it had
      lines.splice(at, blockEnd(lines, at, end) - at, line)
    }
    return lines.join(eol)
  }
  const line = `  ${second}: ${format(value)}`
  if (at === -1) {
    lines.splice(end, 0, `${first}:`, line)
    return lines.join(eol)
  }
  const stop = blockEnd(lines, at, end)
  const child = find(lines, at + 1, stop, second, 2)
  if (child === -1) lines.splice(stop, 0, line)
  else lines[child] = line
  return lines.join(eol)
}
