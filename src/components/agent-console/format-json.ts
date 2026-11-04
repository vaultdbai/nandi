/**
 * Pretty-prints tool arguments/results for display. Strings that contain
 * JSON (e.g. structured tool output) are parsed and re-indented; anything
 * else is shown as-is.
 */
export function formatJson(value: unknown): string {
  if (typeof value === 'string') {
    try {
      const parsed: unknown = JSON.parse(value)
      if (parsed !== null && typeof parsed === 'object') {
        return JSON.stringify(parsed, null, 2)
      }
    } catch {
      // Not JSON — show the raw text
    }
    return value
  }
  return JSON.stringify(value, null, 2) ?? String(value)
}
