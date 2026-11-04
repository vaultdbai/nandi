import { useTranslation } from 'react-i18next'
import { Check, CircleHelp, Copy, X } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { useGatewayManifest, useGatewayStatus } from '@/hooks/useGateway'
import { useRelay, useRelayTrust } from '@/hooks/useRelay'
import type { MemberProfile } from '@/lib/bindings'
import {
  askableSteps,
  canReceiveChecks,
  canSendChecks,
  verdict,
  type Check as CheckRow,
  type CheckStatus,
} from './member-diagnosis'

const ICON: Record<CheckStatus, React.ReactNode> = {
  pass: <Check className="text-success" />,
  fail: <X className="text-destructive" />,
  unknown: <CircleHelp className="text-muted-foreground" />,
}

/**
 * Both directions, one line per opt-in, and for what they could fix a
 * plain message to paste anywhere (network.md §4.3)
 */
export function MemberDiagnosis({ profile }: { profile: MemberProfile }) {
  const { t } = useTranslation()
  const relay = useRelay().status
  const relayTrust = useRelayTrust().data
  const gateway = useGatewayStatus().data
  const owner = useGatewayManifest().data?.owner
  const name = profile.displayName || t('network.unnamed')
  const myReachable = Boolean(
    relay?.connected && gateway?.enabled && gateway.discoverable
  )
  const send = canSendChecks(profile, relay?.instanceId ?? '')
  const receive = canReceiveChecks(
    profile,
    myReachable,
    Boolean(relay?.connected && relayTrust?.enabled && relayTrust.secured)
  )
  const steps = askableSteps(send)

  const copyMessage = () => {
    const me = owner?.githubHandle
      ? `@${owner.githubHandle}${owner.displayName ? ` (${owner.displayName})` : ''}`
      : owner?.displayName || t('network.diagnosis.message.someone')
    const text = [
      t('network.diagnosis.message.hi', { name }),
      ...steps.map(step => t(`network.diagnosis.message.${step}`, { me })),
      t('network.diagnosis.message.thanks'),
    ].join(' ')
    void navigator.clipboard?.writeText(text)
    toast.success(t('network.copied'))
  }

  return (
    <div className="grid gap-4 md:grid-cols-2" data-testid="member-diagnosis">
      <CheckList
        title={t('network.diagnosis.canSend', { name })}
        checks={send}
        label={key => t(`network.diagnosis.send.${key}`, { name })}
        footer={
          steps.length > 0 && (
            <Button size="sm" variant="outline" onClick={copyMessage}>
              <Copy />
              {t('network.diagnosis.copyMessage', { name })}
            </Button>
          )
        }
      />
      <CheckList
        title={t('network.diagnosis.canReceive', { name })}
        checks={receive}
        label={key => t(`network.diagnosis.receive.${key}`, { name })}
      />
    </div>
  )
}

function CheckList<K extends string>({
  title,
  checks,
  label,
  footer,
}: {
  title: string
  checks: CheckRow<K>[]
  label: (key: string) => string
  footer?: React.ReactNode
}) {
  const { t } = useTranslation()
  const yes = verdict(checks)
  return (
    <section className="flex flex-col gap-2 rounded-md border p-3">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">{title}</h3>
        <span
          className={yes ? 'text-sm text-success' : 'text-sm text-destructive'}
        >
          {yes ? t('network.diagnosis.yes') : t('network.diagnosis.no')}
        </span>
      </div>
      <ul className="flex flex-col gap-1.5">
        {checks.map(check => (
          <li
            key={check.key}
            className="flex items-start gap-2 text-sm"
            data-status={check.status}
          >
            <span aria-hidden className="mt-0.5 shrink-0 [&_svg]:size-4">
              {ICON[check.status]}
            </span>
            <span>{label(`${check.key}.${check.status}`)}</span>
          </li>
        ))}
      </ul>
      {footer && <div className="pt-1">{footer}</div>}
    </section>
  )
}
