import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Smartphone } from 'lucide-react'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import {
  useConfirmDevice,
  useOwnDevices,
  useRevokeAllDevices,
  useRevokeDevice,
  useStartPairing,
  useUnlockDevice,
} from '@/hooks/useOwnDevices'
import type { DeviceView, PairingView } from '@/lib/bindings'
import { isRelayNeedsUpdate, isRelayUnavailable } from '@/lib/friendly-error'
import { formatRelativeTime } from '@/lib/format-time'
import { toastRelayActionError } from '@/lib/toast-error'
import { SettingsSectionFrame } from '../SettingsFrame'
import {
  pairedDevices,
  pairingDevice,
  pinProblem,
  statusKey,
} from './devices-model'

/**
 * Settings › Your devices: pair a phone by QR code, compare its key
 * fingerprint, choose its PIN; then the list, with unlock and revoke.
 * Needs a relay that speaks own devices; otherwise the calm notice.
 */
export function DevicesSection() {
  const { t } = useTranslation()
  const devices = useOwnDevices()
  const data = devices.data

  let body: React.ReactNode
  if (devices.isPending) {
    body = <Spinner />
  } else if (
    devices.error &&
    !isRelayUnavailable(devices.error) &&
    !isRelayNeedsUpdate(devices.error)
  ) {
    body = <p className="text-sm text-destructive">{devices.error.message}</p>
  } else if (!data?.connected || data.relayNeedsUpdate) {
    body = (
      <RelayConnectNotice
        reason={
          data?.relayNeedsUpdate || isRelayNeedsUpdate(devices.error)
            ? 'update'
            : 'connect'
        }
      />
    )
  } else {
    body = (
      <>
        <PairPhone devices={data.devices} />
        <DeviceList devices={pairedDevices(data.devices)} />
        <WhatAPhoneCanDo />
      </>
    )
  }

  return (
    <SettingsSectionFrame
      title={t('settings.section.devices')}
      description={t('devices.description')}
    >
      {body}
    </SettingsSectionFrame>
  )
}

/** Name, QR code, then the fingerprint and the PIN */
function PairPhone({ devices }: { devices: DeviceView[] }) {
  const { t } = useTranslation()
  const [name, setName] = useState('')
  const [pairing, setPairing] = useState<PairingView | null>(null)
  const start = useStartPairing()
  const claimed = pairingDevice(devices, pairing?.deviceId)

  if (!pairing) {
    return (
      <form
        className="flex flex-wrap items-end gap-3 rounded-md border p-4"
        onSubmit={event => {
          event.preventDefault()
          start.mutate(name, {
            onSuccess: setPairing,
            onError: toastRelayActionError,
          })
        }}
      >
        <div className="flex min-w-[200px] flex-1 flex-col gap-1.5">
          <Label htmlFor="device-name">{t('devices.nameLabel')}</Label>
          <Input
            id="device-name"
            value={name}
            maxLength={40}
            placeholder={t('devices.namePlaceholder')}
            onChange={event => setName(event.target.value)}
          />
        </div>
        <Button type="submit" disabled={start.isPending}>
          <Smartphone />
          {t('devices.pair')}
        </Button>
      </form>
    )
  }

  const done = () => {
    setPairing(null)
    setName('')
  }
  return (
    <section className="flex flex-col gap-4 rounded-md border p-4">
      {claimed?.status === 'claimed' && claimed.fingerprint ? (
        <ConfirmPhone
          deviceId={claimed.deviceId}
          fingerprint={claimed.fingerprint}
          onDone={done}
        />
      ) : (
        <div className="flex flex-col items-center gap-3">
          <img
            className="size-60 rounded-md bg-background"
            alt={t('devices.qrAlt')}
            src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(pairing.qrSvg)}`}
          />
          <p className="text-center text-sm text-muted-foreground">
            {t('devices.scan', {
              time: formatRelativeTime(pairing.expiresAt, navigator.language),
            })}
          </p>
          <p className="text-sm">{t('devices.waiting')}</p>
        </div>
      )}
      <Button variant="ghost" className="self-start" onClick={done}>
        {t('devices.cancel')}
      </Button>
    </section>
  )
}

function ConfirmPhone({
  deviceId,
  fingerprint,
  onDone,
}: {
  deviceId: string
  fingerprint: string
  onDone: () => void
}) {
  const { t } = useTranslation()
  const [pin, setPin] = useState('')
  const [again, setAgain] = useState('')
  const confirm = useConfirmDevice()
  const problem = pinProblem(pin, again)

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={event => {
        event.preventDefault()
        if (problem) return
        confirm.mutate(
          { deviceId, pin },
          { onSuccess: onDone, onError: toastRelayActionError }
        )
      }}
    >
      <h3 className="font-medium">{t('devices.compareTitle')}</h3>
      <p className="text-sm text-muted-foreground">{t('devices.compare')}</p>
      <p
        className="font-mono text-2xl tracking-widest"
        data-testid="device-fingerprint"
      >
        {fingerprint}
      </p>
      <div className="flex flex-wrap gap-3">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="device-pin">{t('devices.pinLabel')}</Label>
          <Input
            id="device-pin"
            type="password"
            inputMode="numeric"
            autoComplete="new-password"
            value={pin}
            onChange={event => setPin(event.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="device-pin-again">{t('devices.pinAgain')}</Label>
          <Input
            id="device-pin-again"
            type="password"
            inputMode="numeric"
            autoComplete="new-password"
            value={again}
            onChange={event => setAgain(event.target.value)}
          />
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        {pin && problem ? t(problem) : t('devices.pinHint')}
      </p>
      <Button
        type="submit"
        className="self-start"
        disabled={!!problem || confirm.isPending}
      >
        {t('devices.confirm')}
      </Button>
    </form>
  )
}

function DeviceList({ devices }: { devices: DeviceView[] }) {
  const { t } = useTranslation()
  const revoke = useRevokeDevice()
  const revokeAll = useRevokeAllDevices()
  const unlock = useUnlockDevice()

  if (devices.length === 0) {
    return <p className="text-sm text-muted-foreground">{t('devices.empty')}</p>
  }
  return (
    <section className="flex flex-col gap-2">
      <ul className="flex flex-col divide-y rounded-md border">
        {devices.map(device => (
          <li
            key={device.deviceId}
            className="flex flex-wrap items-center gap-3 p-3"
          >
            <div className="min-w-0 flex-1">
              <p className="flex items-center gap-2 font-medium">
                {device.name}
                <Badge
                  variant={
                    device.status === 'locked' ? 'destructive' : 'secondary'
                  }
                >
                  {t(statusKey(device.status))}
                </Badge>
                {device.online && (
                  <Badge variant="outline">{t('devices.online')}</Badge>
                )}
              </p>
              <p className="text-xs text-muted-foreground">
                {device.pairedAt !== null &&
                  t('devices.paired', {
                    time: formatRelativeTime(
                      device.pairedAt,
                      navigator.language
                    ),
                  })}
                {' · '}
                {device.lastSeenAt === null
                  ? t('devices.neverSeen')
                  : t('devices.lastSeen', {
                      time: formatRelativeTime(
                        device.lastSeenAt,
                        navigator.language
                      ),
                    })}
              </p>
              {device.status === 'locked' && (
                <p className="text-xs text-destructive">
                  {t('devices.lockedHint')}
                </p>
              )}
            </div>
            {device.status === 'locked' && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => unlock.mutate(device.deviceId)}
              >
                {t('devices.unlock')}
              </Button>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                revoke.mutate(device.deviceId, {
                  onError: toastRelayActionError,
                })
              }
            >
              {t('devices.revoke')}
            </Button>
          </li>
        ))}
      </ul>
      <Button
        size="sm"
        variant="ghost"
        className="self-end text-destructive"
        onClick={() => {
          if (window.confirm(t('devices.revokeAllConfirm'))) {
            revokeAll.mutate(undefined, { onError: toastRelayActionError })
          }
        }}
      >
        {t('devices.revokeAll')}
      </Button>
    </section>
  )
}

function WhatAPhoneCanDo() {
  const { t } = useTranslation()
  return (
    <section className="grid gap-3 text-sm sm:grid-cols-2">
      <div>
        <h3 className="font-medium">{t('devices.canTitle')}</h3>
        <p className="text-muted-foreground">{t('devices.can')}</p>
      </div>
      <div>
        <h3 className="font-medium">{t('devices.cannotTitle')}</h3>
        <p className="text-muted-foreground">{t('devices.cannot')}</p>
      </div>
    </section>
  )
}
