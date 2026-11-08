import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { BackfillChoice } from '@/components/email/BackfillChoice'
import { ConnectMailbox } from '@/components/email/ConnectMailbox'
import { EmailReads } from '@/components/email/EmailReads'
import {
  DEFAULT_INTERVAL_MINUTES,
  intervalLabel,
  intervalOptions,
  mailboxCard,
} from '@/components/settings/email/email-model'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Spinner } from '@/components/ui/spinner'
import { useEmailAction, useEmailStatus } from '@/hooks/useEmail'
import type {
  BackfillRange,
  EmailProviderStatus,
  MailProvider,
} from '@/lib/tauri-bindings'

export interface StepEmailProps {
  onNext: () => void
}

/**
 * Onboarding step 1 (skippable): connect the work mailbox. One sign-in
 * covers which mailboxes the Chief of Staff reads (Outlook, Gmail or both,
 * each connected in turn), how often it checks (30 minutes unless the user
 * picks another) and who the user is: the name step that follows starts
 * from the mailbox. Skipping leaves a to-do on Home and the name step
 * asks. A provider this build can't connect says so and doesn't block.
 */
export function StepEmail({ onNext }: StepEmailProps) {
  const { t } = useTranslation()
  const status = useEmailStatus()
  const action = useEmailAction()
  const [chosen, setChosen] = useState<MailProvider[]>([])
  const [intervalEdit, setIntervalEdit] = useState<number | null>(null)
  const providers = status.data?.providers ?? []
  const connected = providers.filter(p => p.connected)
  // A fresh install: 30 minutes; coming back: what was saved
  const savedInterval =
    status.data && status.data.providers.some(p => p.connected)
      ? status.data.intervalMinutes
      : DEFAULT_INTERVAL_MINUTES
  const interval = intervalEdit ?? savedInterval
  const [backfillEdit, setBackfillEdit] = useState<BackfillRange | null>(null)
  const backfill = backfillEdit ?? status.data?.backfillRange ?? 'threeMonths'

  const toggle = (provider: MailProvider, on: boolean) =>
    setChosen(current =>
      on
        ? [...current.filter(p => p !== provider), provider]
        : current.filter(p => p !== provider)
    )

  const next = () => {
    action.mutate(
      {
        kind: 'settings',
        settings: {
          intervalMinutes: interval,
          watch: status.data?.watch ?? [],
          backfill,
        },
      },
      // Setup goes on: the interval can be changed in Settings › Email
      { onSettled: onNext }
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">{t('onboarding.email.title')}</h2>
        <p className="text-sm text-muted-foreground">
          {t('onboarding.email.description')}
        </p>
      </div>

      <EmailReads />

      {status.isPending ? (
        <Spinner />
      ) : (
        <fieldset className="flex flex-col gap-3">
          <legend className="mb-2 text-sm font-medium">
            {t('onboarding.email.which')}
          </legend>
          {providers.map(provider => (
            <MailboxChoice
              key={provider.provider}
              status={provider}
              chosen={chosen.includes(provider.provider)}
              onToggle={on => toggle(provider.provider, on)}
            />
          ))}
        </fieldset>
      )}

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-2 text-sm font-medium">
          {t('onboarding.email.often')}
        </legend>
        <RadioGroup
          value={String(interval)}
          onValueChange={value => setIntervalEdit(Number(value))}
          className="flex flex-wrap gap-2"
        >
          {intervalOptions(interval).map(minutes => {
            const id = `onboarding-email-every-${minutes}`
            return (
              <div
                key={minutes}
                className="flex items-center gap-2 rounded-md border px-3 py-2"
              >
                <RadioGroupItem id={id} value={String(minutes)} />
                <Label htmlFor={id}>{intervalLabel(t, minutes)}</Label>
              </div>
            )
          })}
        </RadioGroup>
        <p className="text-xs text-muted-foreground">{t('email.oneRun')}</p>
      </fieldset>

      <BackfillChoice
        value={backfill}
        onChange={setBackfillEdit}
        connected={connected.length > 0}
        idPrefix="onboarding-email"
      />

      <div className="flex justify-end gap-2">
        <Button disabled={action.isPending} onClick={next}>
          {action.isPending && <Spinner />}
          {connected.length === 0
            ? t('onboarding.email.skip')
            : t('onboarding.continue')}
        </Button>
      </div>
    </div>
  )
}

function MailboxChoice({
  status,
  chosen,
  onToggle,
}: {
  status: EmailProviderStatus
  chosen: boolean
  onToggle: (on: boolean) => void
}) {
  const { t } = useTranslation()
  const card = mailboxCard(status)
  const id = `onboarding-email-${status.provider}`
  return (
    <div
      className="flex flex-col gap-3 rounded-md border p-3"
      data-provider={status.provider}
    >
      <div className="flex items-start gap-3">
        <Checkbox
          id={id}
          checked={chosen || status.connected}
          disabled={!status.available || status.connected}
          onCheckedChange={checked => onToggle(checked === true)}
          className="mt-0.5"
        />
        <div className="space-y-0.5">
          <Label htmlFor={id}>{card.name}</Label>
          <p className="text-xs text-muted-foreground">
            {card.account
              ? t('email.connectedAs', { account: card.account })
              : t(card.statusKey)}
          </p>
        </div>
      </div>
      {chosen && status.available && !status.connected && (
        <div className="ps-7">
          <ConnectMailbox provider={status.provider} />
        </div>
      )}
    </div>
  )
}
