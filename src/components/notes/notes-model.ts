/**
 * My notes, pure: grouping for the list, and the note name rules (the same
 * as `services::notes::check_name`, so a bad name is caught before saving).
 */
import type { NoteFile } from '@/lib/bindings'

export interface NoteGroup {
  /** `''` for notes at the top of the folder */
  folder: string
  notes: NoteFile[]
}

/** Top-level notes first, then each folder's, all sorted by name */
export function groupNotes(notes: readonly NoteFile[]): NoteGroup[] {
  const groups = new Map<string, NoteFile[]>()
  for (const note of [...notes].sort((a, b) => a.name.localeCompare(b.name))) {
    const slash = note.name.lastIndexOf('/')
    const folder = slash === -1 ? '' : note.name.slice(0, slash)
    groups.set(folder, [...(groups.get(folder) ?? []), note])
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b)))
    .map(([folder, list]) => ({ folder, notes: list }))
}

/** What the list shows: the note's heading, else its file name */
export function noteLabel(note: NoteFile): string {
  const file = note.name.slice(note.name.lastIndexOf('/') + 1)
  return note.title ?? file.replace(/\.md$/i, '')
}

const MAX_NAME_CHARS = 120
const MAX_DEPTH = 3
const PART = /^[\p{L}\p{N}_.\- ]+$/u

/**
 * The note name typed for a new note: `.md` added when missing, slashes
 * kept for folders. `null` when it can't be a note name.
 */
export function newNoteName(typed: string): string | null {
  let name = typed.trim().replace(/\\/g, '/')
  if (!name) return null
  if (!/\.md$/i.test(name)) name = `${name}.md`
  const parts = name.split('/')
  const valid =
    name.length <= MAX_NAME_CHARS &&
    parts.length <= MAX_DEPTH &&
    parts.every(
      part =>
        part.length > 0 &&
        PART.test(part) &&
        !part.startsWith('.') &&
        !part.startsWith(' ') &&
        !part.endsWith(' ')
    ) &&
    (parts.at(-1)?.length ?? 0) > 3
  return valid ? name : null
}

/** A new note's first line: its name as a heading */
export function newNoteContent(name: string): string {
  const file = name.slice(name.lastIndexOf('/') + 1).replace(/\.md$/i, '')
  const title = file.replace(/[-_]+/g, ' ').trim()
  return `# ${title.charAt(0).toUpperCase()}${title.slice(1)}\n\n`
}
