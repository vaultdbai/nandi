import { useTranslation } from 'react-i18next'
import { KeyRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useAcceptPeerKey } from '@/hooks/useMemberProfile'
import type { PeerKeyStatus } from '@/lib/bindings'
import { toastError } from '@/lib/toast-error'

export interface MemberKeyNoticeProps {
  instanceId: string
  name: string
  peerKey: PeerKeyStatus | null
}

/**
 * The member's key on their profile: its fingerprint, and when the relay
 * shows a new one, a calm "Their key changed" with the new fingerprint and
 * an accept button. Sending to them waits until it's accepted.
 */
export function MemberKeyNotice({
  instanceId,
  name,
  peerKey,
}: MemberKeyNoticeProps) {
  const { t } = useTranslation()
  const accept = useAcceptPeerKey()
  if (!peerKey) return null

  if (!peerKey.changedFingerprint) {
    return (
      <p
        className="flex items-center gap-2 text-xs text-muted-foreground"
        data-testid="member-key"
      >
        <KeyRound className="size-3.5" />
        {t('members.key.fingerprint', { fingerprint: peerKey.fingerprint })}
      </p>
    )
  }

  return (
    <div
      className="flex flex-wrap items-center gap-3 rounded-md border bg-muted/40 px-3 py-2"
      data-testid="member-key-changed"
    >
      <KeyRound className="size-4 text-muted-foreground" />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="text-sm font-medium">{t('members.key.changedTitle')}</p>
        <p className="text-sm text-muted-foreground">
          {t('members.key.changedDescription', { name })}
        </p>
        <p className="font-mono text-xs text-muted-foreground">
          {t('members.key.changedFingerprints', {
            before: peerKey.fingerprint,
            after: peerKey.changedFingerprint,
          })}
        </p>
      </div>
      <Button
        size="sm"
        variant="outline"
        disabled={accept.isPending}
        onClick={() =>
          accept.mutate(instanceId, {
            onError: error => toastError(error, t('members.key.acceptFailed')),
          })
        }
      >
        {t('members.key.accept')}
      </Button>
    </div>
  )
}
