/**
 * Who AWS says you are, without the full ARN on screen: the user or role
 * name, and the account as its last 4 digits
 */

/** `arn:aws:iam::123:user/publisher` → `publisher`; `null` if none */
export function arnName(arn: string | null): string | null {
  if (!arn) return null
  const resource = arn.split(':').slice(5).join(':')
  const parts = resource.split('/').filter(Boolean)
  // An assumed role ends in its session name; the role is the part before
  if (resource.startsWith('assumed-role/') && parts.length >= 2) {
    return parts[1] ?? null
  }
  return parts.at(-1) ?? (resource || null)
}

/** `440955376164` → `••••6164` */
export function maskAccount(accountId: string | null): string | null {
  if (!accountId) return null
  return `••••${accountId.slice(-4)}`
}

/** "publisher · account ••••6164 · us-east-2", parts left out when unknown */
export function maskedIdentity(
  status: {
    arn: string | null
    accountId: string | null
    region: string | null
  },
  accountLabel: (masked: string) => string
): string {
  const account = maskAccount(status.accountId)
  return [arnName(status.arn), account && accountLabel(account), status.region]
    .filter(Boolean)
    .join(' · ')
}
