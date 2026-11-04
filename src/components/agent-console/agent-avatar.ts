/** Avatar fallback palettes (background + text), picked per agent by ID */
const AVATAR_PALETTES = [
  'bg-violet-500/15 text-violet-700 dark:text-violet-300',
  'bg-sky-500/15 text-sky-700 dark:text-sky-300',
  'bg-rose-500/15 text-rose-700 dark:text-rose-300',
  'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300',
  'bg-amber-500/15 text-amber-700 dark:text-amber-300',
  'bg-indigo-500/15 text-indigo-700 dark:text-indigo-300',
] as const

/** Up to two initials from the agent name, e.g. "Research Agent" → "RA" */
export function getAgentInitials(name: string): string {
  const words = name.split(/[\s_-]+/).filter(Boolean)
  const initials =
    words.length > 1
      ? words.slice(0, 2).map(word => word.charAt(0))
      : Array.from(words[0] ?? '?').slice(0, 2)
  return initials.join('').toUpperCase()
}

/** Stable color for an agent, so it keeps its color across reloads */
export function getAgentAvatarClassName(agentId: string): string {
  let hash = 0
  for (const char of agentId) {
    hash = (hash * 31 + char.charCodeAt(0)) | 0
  }
  return AVATAR_PALETTES[Math.abs(hash) % AVATAR_PALETTES.length] ?? ''
}
