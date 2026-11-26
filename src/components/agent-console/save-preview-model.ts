/**
 * The card for saving a definition (`nandi_save_artifact`), Nandi's or
 * Ask's: Create approves the save, No declines it, Edit opens the file in
 * the editor instead. Pure; `SavePreviewSection` renders it.
 */
import type { EditPreview, JsonValue } from '@/lib/bindings'

/** `studio::save::SAVE_ARTIFACT` */
export const SAVE_TOOL = 'nandi_save_artifact'

/** The kinds the editor opens as a file */
const FILE_KINDS = ['agent', 'workflow', 'capability', 'policy'] as const
type FileKind = (typeof FILE_KINDS)[number]

const isFileKind = (kind: unknown): kind is FileKind =>
  typeof kind === 'string' && (FILE_KINDS as readonly string[]).includes(kind)

/**
 * The call's file as a draft for the editor, or `null` when it has none
 * the editor opens (a trigger, a connector), or no ID to open it by
 */
export function saveDraft(
  args: JsonValue,
  id: string | null = null
): EditPreview | null {
  if (typeof args !== 'object' || args === null || Array.isArray(args)) {
    return null
  }
  const { kind, content } = args as Record<string, JsonValue>
  const named = id ?? (typeof args.id === 'string' ? args.id : null)
  if (!isFileKind(kind) || typeof content !== 'string' || !named) return null
  return {
    artifact: { kind, id: named },
    before: '',
    after: content,
    diff: '',
    impact: [],
    versionBump: null,
  }
}
