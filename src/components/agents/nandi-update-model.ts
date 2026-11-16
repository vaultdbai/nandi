/**
 * The model behind Nandi's update card: how a shipped file is named to the
 * person. Pure, so it's testable without React.
 */
import { DEFAULT_AGENT } from '@/lib/bindings'

/** Nandi's folder in the gold copy */
const AGENT_FOLDER = `.nandi/agents/${DEFAULT_AGENT}/`
/** Where everything else Nandi ships lives (its default triggers) */
const META_FOLDER = '.nandi/'

/**
 * A shipped file by the name the person knows it by: `instructions.md`,
 * `skills/occasions.md`, `triggers/morning-digest.yaml`
 */
export function shippedFileName(path: string): string {
  if (path.startsWith(AGENT_FOLDER)) return path.slice(AGENT_FOLDER.length)
  if (path.startsWith(META_FOLDER)) return path.slice(META_FOLDER.length)
  return path
}
