import type { HarnessAuth, HarnessInfo } from '@/lib/bindings'

/** `native` plus every known harness, keeping an unknown saved value */
export function runtimeOptions(
  harnesses: { id: string; displayName: string; installed: boolean }[],
  value: string
): { id: string; label: string; installed: boolean }[] {
  const options = [
    { id: 'native', label: 'Nandi', installed: true },
    ...harnesses.map(h => ({
      id: h.id,
      label: h.displayName,
      installed: h.installed,
    })),
  ]
  if (value && !options.some(o => o.id === value)) {
    options.push({ id: value, label: value, installed: false })
  }
  return options
}

/** One pill per runtime card */
export type RuntimeStatus =
  | 'ready'
  | 'notFound'
  | 'needsSandbox'
  | 'needsSignIn'
  | 'checking'
  | 'couldntCheck'

/**
 * Agents run only in the sandbox container, so a runtime is ready when the
 * sandbox image is built and the sign-in reaches it. On this computer the
 * CLI is only where the user signs in.
 */
export function runtimeStatus(harness: HarnessInfo): RuntimeStatus {
  const { health } = harness
  if (!health.installed && !harness.imageAvailable) return 'notFound'
  if (!harness.imageAvailable) return 'needsSandbox'
  return health.containerAuthenticated ? 'ready' : 'needsSignIn'
}

/** `enterprise` → `Enterprise` */
const titleCase = (value: string) =>
  value.charAt(0).toUpperCase() + value.slice(1)

/**
 * The sign-in line's translation key and values, e.g. "Account sign-in
 * (Enterprise · Acme)" or "AWS Bedrock (acme-dev · us-east-1)"
 */
export function authSummary(auth: HarnessAuth | null | undefined): {
  key: string
  values: { details: string }
} {
  if (!auth || auth.kind === 'none') {
    return { key: 'harness.auth.none', values: { details: '' } }
  }
  const parts =
    auth.kind === 'subscription'
      ? [auth.plan ? titleCase(auth.plan) : null, auth.organization]
      : auth.kind === 'bedrock'
        ? [auth.awsProfile, auth.awsRegion]
        : []
  const details = parts.filter(Boolean).join(' · ')
  return {
    key: details
      ? `harness.auth.${auth.kind}With`
      : `harness.auth.${auth.kind}`,
    values: { details },
  }
}

/** The model provider whose key lets a runtime sign in inside a container */
export function keyProviderFor(harnessId: string): string | undefined {
  if (harnessId === 'claude-code') return 'anthropic'
  if (harnessId === 'codex') return 'openAi'
  return undefined
}
