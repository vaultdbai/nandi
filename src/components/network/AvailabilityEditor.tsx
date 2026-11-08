/**
 * Your availability (Network › Sharing › What you offer): available, busy
 * or away, until which day, and who covers. Saved to `availability:` in
 * NANDI.md and shown on your card.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { DatePicker } from '@/components/ui/date-picker'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { errorMessage } from '@/hooks/useDelegation'
import { useSetGatewayManifest } from '@/hooks/useGateway'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import { useOwnInstanceId } from '@/hooks/useRelay'
import type { GatewayManifest } from '@/lib/bindings'
import {
  availabilityOf,
  choiceFrom,
  type AvailabilityChoice,
} from './availability-model'

const STATUSES: AvailabilityChoice['status'][] = ['available', 'busy', 'away']

export function AvailabilityEditor({
  manifest,
}: {
  manifest: GatewayManifest
}) {
  const { t } = useTranslation()
  const save = useSetGatewayManifest()
  const snapshot = useNetworkSnapshot()
  const ownId = useOwnInstanceId()
  const [choice, setChoice] = useState(() => choiceFrom(manifest.availability))
  const others = (snapshot.data?.members ?? []).filter(
    member => member.instanceId !== ownId
  )
  const away = choice.status !== 'available'

  const submit = () =>
    save.mutate(
      { ...manifest, availability: availabilityOf(choice) },
      {
        onSuccess: () => toast.success(t('network.availability.saved')),
        onError: error => toast.error(errorMessage(error)),
      }
    )

  return (
    <section
      className="flex flex-col gap-3 rounded-md border p-3"
      aria-labelledby="availability-title"
      data-testid="availability-editor"
    >
      <div className="grid gap-1">
        <h2 id="availability-title" className="text-sm font-semibold">
          {t('network.availability.title')}
        </h2>
        <p className="text-xs text-muted-foreground">
          {t('network.availability.hint')}
        </p>
      </div>
      <ToggleGroup
        type="single"
        variant="outline"
        size="sm"
        className="flex-wrap self-start"
        value={choice.status}
        onValueChange={value => {
          if (value)
            setChoice(current => ({
              ...current,
              status: value as AvailabilityChoice['status'],
            }))
        }}
        aria-label={t('network.availability.status')}
      >
        {STATUSES.map(status => (
          <ToggleGroupItem key={status} value={status}>
            {t(`network.availability.${status}`)}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      {away && (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="grid gap-2">
            <Label>{t('network.availability.until')}</Label>
            <DatePicker
              value={choice.until}
              onChange={until => setChoice(current => ({ ...current, until }))}
              placeholder={t('network.availability.untilNone')}
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="availability-covers">
              {t('network.availability.covers')}
            </Label>
            <NativeSelect
              id="availability-covers"
              value={choice.delegateTo}
              onChange={event =>
                setChoice(current => ({
                  ...current,
                  delegateTo: event.target.value,
                }))
              }
            >
              <NativeSelectOption value="">
                {t('network.availability.nobody')}
              </NativeSelectOption>
              {others.map(member => (
                <NativeSelectOption
                  key={member.instanceId}
                  value={member.instanceId}
                >
                  {member.displayName || member.instanceId}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
        </div>
      )}
      <Button
        size="sm"
        className="self-end"
        onClick={submit}
        disabled={save.isPending}
      >
        {save.isPending && <Spinner />}
        {t('network.availability.save')}
      </Button>
    </section>
  )
}
