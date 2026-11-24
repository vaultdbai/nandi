/**
 * Settings › Your devices: the pure rules the view follows. The Nandi
 * checks the PIN again (`services::own_devices::pin`); this only says
 * early what it will refuse.
 */
import type { DeviceStatus, DeviceView } from '@/lib/bindings'

export const MIN_PIN_DIGITS = 6
export const MAX_PIN_DIGITS = 12

/** The i18n key of why `pin` and `again` can't be confirmed, or `null` */
export function pinProblem(pin: string, again: string): string | null {
  const digits = new RegExp(`^\\d{${MIN_PIN_DIGITS},${MAX_PIN_DIGITS}}$`)
  if (!digits.test(pin)) return 'devices.pinDigits'
  if (pin !== again) return 'devices.pinMismatch'
  return null
}

/** The i18n key of a device's status */
export function statusKey(status: DeviceStatus): string {
  return `devices.status.${status}`
}

/** Paired devices first (newest pairing last), then pending ones */
export function pairedDevices(devices: DeviceView[]): DeviceView[] {
  return devices.filter(d => d.status === 'active' || d.status === 'locked')
}

/** The pairing the screen follows, as the relay last told it */
export function pairingDevice(
  devices: DeviceView[],
  deviceId: string | undefined
): DeviceView | undefined {
  return deviceId ? devices.find(d => d.deviceId === deviceId) : undefined
}
