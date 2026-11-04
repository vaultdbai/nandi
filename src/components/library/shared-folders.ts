/**
 * Shared folders, as the filesystem connector's config holds them
 * (`services/connectors/folders.rs`): a list of `{ id, path, access }`.
 * An agent gets only the folders it names. Pure.
 */

export type FolderAccess = 'read' | 'write'

export interface SharedFolder {
  id: string
  path: string
  access: FolderAccess
}

const MAX_ID_CHARS = 40

/** The list in a filesystem connector's config */
export function foldersOf(
  config: Record<string, unknown> | undefined
): SharedFolder[] {
  const list = config?.folders
  if (!Array.isArray(list)) return []
  return list.flatMap(item => {
    if (typeof item !== 'object' || item === null) return []
    const { id, path, access } = item as Record<string, unknown>
    if (typeof id !== 'string' || typeof path !== 'string') return []
    return [{ id, path, access: access === 'write' ? 'write' : 'read' }]
  })
}

/** The connector config that saves `folders` */
export function folderConfig(folders: SharedFolder[]): {
  folders: { id: string; path: string; access: FolderAccess }[]
} {
  return {
    folders: folders.map(({ id, path, access }) => ({
      id: id.trim(),
      path: path.trim(),
      access,
    })),
  }
}

export function validFolderId(id: string): boolean {
  return (
    id.length > 0 &&
    id.length <= MAX_ID_CHARS &&
    /^[a-z0-9][a-z0-9_-]*$/.test(id)
  )
}

/** A short ID from a folder's path ("~/Work Notes" → "work-notes"), not in `taken` */
export function folderIdFrom(path: string, taken: readonly string[]): string {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? ''
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, MAX_ID_CHARS - 3) || 'folder'
  let id = base
  for (let n = 2; taken.includes(id); n++) id = `${base}-${n}`
  return id
}

export type FolderProblem = 'noPath' | 'badId' | 'duplicateId'

/** What's wrong with the list before it can be saved, per row */
export function folderProblems(
  folders: SharedFolder[]
): (FolderProblem | null)[] {
  return folders.map((folder, index) => {
    if (!folder.path.trim()) return 'noPath'
    if (!validFolderId(folder.id.trim())) return 'badId'
    const first = folders.findIndex(f => f.id.trim() === folder.id.trim())
    return first !== index ? 'duplicateId' : null
  })
}

/** A folder an agent gets, and the access it asks for (`null`: the folder's own) */
export interface FolderChoice {
  id: string
  access?: FolderAccess | null
}

/** The access an agent ends up with: never more than the folder's */
export function effectiveAccess(
  folder: SharedFolder,
  choice: FolderChoice
): FolderAccess {
  return folder.access === 'write' && choice.access !== 'read'
    ? 'write'
    : 'read'
}

/**
 * The agent's folder list after turning `folderId` on or off, or setting
 * its access. Write on a read-only folder is stored as the folder's own
 * access (`null`), which stays read.
 */
export function chooseFolder(
  choices: readonly FolderChoice[],
  folder: SharedFolder,
  next: 'off' | FolderAccess
): FolderChoice[] {
  const rest = choices.filter(c => c.id !== folder.id)
  if (next === 'off') return rest
  const access: FolderAccess | null =
    next === 'read' && folder.access === 'write' ? 'read' : null
  const updated: FolderChoice = { id: folder.id, access }
  const at = choices.findIndex(c => c.id === folder.id)
  if (at === -1) return [...choices, updated]
  return choices.map(c => (c.id === folder.id ? updated : c))
}
