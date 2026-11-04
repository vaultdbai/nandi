import { useTranslation } from 'react-i18next'
import { ShieldCheck } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { Switch } from '@/components/ui/switch'
import { useMemberProfile } from '@/hooks/useMemberProfile'
import {
  isMemberBlocked,
  useBlockMember,
  usePauseDelegationsFromMember,
  useSetTrustRelationship,
} from '@/hooks/useTrust'
import type { TrustRelationship } from '@/lib/bindings'
import { cn } from '@/lib/utils'

export interface TrustControlsProps {
  instanceId: string
  /** The current relationship; fetched (with the profile) when omitted */
  trust?: TrustRelationship
  /** Explains each toggle under its label */
  showHints?: boolean
  className?: string
}

/** Trusted peer, pause delegations, and block for one member, as switches */
export function TrustControls({
  instanceId,
  trust,
  showHints = false,
  className,
}: TrustControlsProps) {
  const { t } = useTranslation()
  // Only fetched when the caller didn't pass the relationship
  const profile = useMemberProfile(trust ? '' : instanceId)
  const current = trust ?? profile.data?.trust
  const setTrust = useSetTrustRelationship()
  const pause = usePauseDelegationsFromMember()
  const block = useBlockMember()
  const busy =
    !current || setTrust.isPending || pause.isPending || block.isPending

  const onError = (error: Error) =>
    toast.error(t('members.trust.failed', { error: error.message }))

  return (
    <div className={cn('flex flex-col gap-3', className)}>
      <TrustSwitch
        id={`trusted-${instanceId}`}
        label={t('members.trust.trustedPeer')}
        hint={showHints ? t('members.trust.trustedPeerHint') : undefined}
        checked={current?.addedAsTrustedPeer ?? false}
        disabled={busy}
        onChange={checked =>
          setTrust.mutate(
            {
              instanceId,
              update: { addedAsTrustedPeer: checked, delegationsPaused: null },
            },
            { onError }
          )
        }
      />
      <TrustSwitch
        id={`paused-${instanceId}`}
        label={t('members.trust.pauseDelegations')}
        hint={showHints ? t('members.trust.pauseDelegationsHint') : undefined}
        checked={current?.delegationsPaused ?? false}
        disabled={busy}
        onChange={paused => pause.mutate({ instanceId, paused }, { onError })}
      />
      <TrustSwitch
        id={`blocked-${instanceId}`}
        label={t('members.trust.blockMember')}
        hint={showHints ? t('members.trust.blockMemberHint') : undefined}
        checked={isMemberBlocked(current)}
        disabled={busy}
        onChange={blocked => block.mutate({ instanceId, blocked }, { onError })}
      />
    </div>
  )
}

/**
 * `TrustControls` behind a "Trust" button: the member's relationship is
 * only fetched once it's opened (the shared grids show many owners)
 */
export function TrustPopover({ instanceId }: { instanceId: string }) {
  const { t } = useTranslation()
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="sm" variant="ghost">
          <ShieldCheck />
          {t('members.trust.title')}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72">
        <TrustControls instanceId={instanceId} />
      </PopoverContent>
    </Popover>
  )
}

function TrustSwitch({
  id,
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  id: string
  label: string
  hint?: string
  checked: boolean
  disabled: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="flex flex-col gap-0.5">
        <Label htmlFor={id} className="text-sm font-normal">
          {label}
        </Label>
        {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      </div>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
      />
    </div>
  )
}
