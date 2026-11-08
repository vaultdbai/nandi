/**
 * Site secrets on an agent's Permissions tab: the names its browser types
 * (`SITE_PASSWORD`), never their values. The backend checks the same rule
 * (`PilotOptions::validated`); this gives the form its message first.
 */

export type SecretNameProblem = 'empty' | 'format' | 'taken'

/** What the user types, as a secret name: capitals, digits and `_` */
export function normalizeSecretName(input: string): string {
  return input
    .trimStart()
    .toUpperCase()
    .replace(/[^A-Z0-9_]+/g, '_')
}

/** Why `name` can't be added next to the `existing` ones, or null */
export function secretNameProblem(
  name: string,
  existing: readonly string[]
): SecretNameProblem | null {
  if (!name) return 'empty'
  if (!/^[A-Z][A-Z0-9_]*$/.test(name)) return 'format'
  if (existing.includes(name)) return 'taken'
  return null
}
