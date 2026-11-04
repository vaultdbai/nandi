/**
 * Rules for setting up AWS profiles in Settings (pure, mirroring the
 * backend's `aws_auth::setup`, so most mistakes show before saving).
 * Each check returns a translation key, or `null` when the value is fine.
 */
import type { AwsCliInfo, AwsProfile, SsoAccount } from '@/lib/bindings'

const PROFILE_NAME = /^[A-Za-z0-9._-]{1,64}$/
const REGION = /^[a-z0-9-]{4,32}$/

export function profileNameError(name: string): string | null {
  return PROFILE_NAME.test(name.trim())
    ? null
    : 'harness.aws.setup.errors.profileName'
}

/** An empty region is fine: the profile keeps (or gets) its default */
export function regionError(region: string): string | null {
  const value = region.trim()
  if (!value) return null
  return REGION.test(value) && value.includes('-')
    ? null
    : 'harness.aws.setup.errors.region'
}

export function startUrlError(url: string): string | null {
  try {
    const parsed = new URL(url.trim())
    return parsed.protocol === 'https:' && parsed.hostname
      ? null
      : 'harness.aws.setup.errors.startUrl'
  } catch {
    return 'harness.aws.setup.errors.startUrl'
  }
}

/** `Prod Account` + `ReadOnly` → `prod-account-readonly` */
export function suggestedProfileName(
  account: SsoAccount,
  role: string
): string {
  return `${account.accountName}-${role}`
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64)
}

/** Whether Log in applies: SSO, or `aws login` where the CLI has it */
export function canLogIn(profile: AwsProfile | null, cli?: AwsCliInfo) {
  if (profile?.kind === 'sso') return true
  return profile?.kind === 'consoleLogin' && !!cli?.supportsLogin
}
