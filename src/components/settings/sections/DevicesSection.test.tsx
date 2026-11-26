import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { DeviceView, DevicesOverview, PairingView } from '@/lib/bindings'
import { DevicesSection } from './DevicesSection'
import { pinProblem } from './devices-model'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const DEVICE = 'device.0123456789abcdef0123456789abcdef'

const device = (patch: Partial<DeviceView>): DeviceView => ({
  deviceId: DEVICE,
  name: "Ada's phone",
  status: 'active',
  fingerprint: '3F2A 91C0 7B44 E1D9',
  pairedAt: Date.now() - 60_000,
  lastSeenAt: null,
  online: false,
  failedPins: 0,
  askLevel: 'confirm',
  ...patch,
})

const overview = (patch: Partial<DevicesOverview>): DevicesOverview => ({
  connected: true,
  relayNeedsUpdate: false,
  devices: [],
  ...patch,
})

const pairing: PairingView = {
  deviceId: DEVICE,
  name: "Ada's phone",
  invite: {
    relayUrl: 'wss://relay.example/relay',
    homeInstance: 'ada-1',
    homeKeys: { signing: 's', sealing: 'x' },
    deviceId: DEVICE,
    code: 'code',
    expiresAt: Date.now() + 300_000,
  },
  link: 'https://vaultdb.ai/pair#abc',
  qrSvg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
  expiresAt: Date.now() + 300_000,
}

describe('pinProblem', () => {
  it('wants 6 to 12 digits, typed the same twice', () => {
    expect(pinProblem('482916', '482916')).toBeNull()
    expect(pinProblem('48291', '48291')).toBe('devices.pinDigits')
    expect(pinProblem('48291a', '48291a')).toBe('devices.pinDigits')
    expect(pinProblem('482916', '482917')).toBe('devices.pinMismatch')
  })
})

describe('Settings › Your devices', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listOwnDevices: ok(overview({})),
      startDevicePairing: ok(pairing),
      confirmOwnDevice: ok(device({})),
      revokeOwnDevice: ok(null),
      revokeAllOwnDevices: ok(null),
      unlockOwnDevice: ok(null),
    })
  })

  it('asks to connect without a relay, and says an old relay needs an update', async () => {
    mocks().listOwnDevices = ok(overview({ connected: false }))
    const { unmount } = render(<DevicesSection />)
    expect(await screen.findByTestId('relay-connect-notice')).toBeVisible()
    expect(screen.queryByText('Pair a phone')).toBeNull()
    unmount()

    mocks().listOwnDevices = ok(overview({ relayNeedsUpdate: true }))
    render(<DevicesSection />)
    expect(await screen.findByTestId('relay-update-notice')).toBeVisible()
  })

  it('pairs: the QR code while it waits for the phone', async () => {
    const user = userEvent.setup()
    render(<DevicesSection />)
    await user.type(await screen.findByLabelText('Name'), "Ada's phone")
    await user.click(screen.getByRole('button', { name: 'Pair a phone' }))
    expect(await screen.findByAltText('Pairing QR code')).toBeVisible()
    expect(mocks().startDevicePairing).toHaveBeenCalledWith("Ada's phone")
    expect(screen.getByText('Waiting for your phone…')).toBeVisible()
  })

  it('shows the claimed phone’s fingerprint and confirms it with a PIN', async () => {
    const user = userEvent.setup()
    render(<DevicesSection />)
    // By the time the list comes back, the phone has claimed the code
    mocks().listOwnDevices = ok(
      overview({ devices: [device({ status: 'claimed', pairedAt: null })] })
    )
    await user.click(
      await screen.findByRole('button', { name: 'Pair a phone' })
    )
    expect(await screen.findByTestId('device-fingerprint')).toHaveTextContent(
      '3F2A 91C0 7B44 E1D9'
    )
    const confirm = screen.getByRole('button', { name: 'Confirm' })
    await user.type(
      screen.getByLabelText('Choose a PIN (6 to 12 digits)'),
      '482916'
    )
    expect(confirm).toBeDisabled()
    await user.type(screen.getByLabelText('PIN again'), '482916')
    await user.click(confirm)
    await waitFor(() =>
      expect(mocks().confirmOwnDevice).toHaveBeenCalledWith(DEVICE, '482916')
    )
  })
  it('lists paired devices with unlock and revoke', async () => {
    const user = userEvent.setup()
    mocks().listOwnDevices = ok(
      overview({ devices: [device({ status: 'locked', failedPins: 5 })] })
    )
    render(<DevicesSection />)
    expect(await screen.findByText("Ada's phone")).toBeVisible()
    expect(screen.getByText('Locked')).toBeVisible()
    await user.click(screen.getByRole('button', { name: 'Unlock' }))
    await waitFor(() =>
      expect(mocks().unlockOwnDevice).toHaveBeenCalledWith(DEVICE, null)
    )
    await user.click(screen.getByRole('button', { name: 'Revoke' }))
    await waitFor(() =>
      expect(mocks().revokeOwnDevice).toHaveBeenCalledWith(DEVICE)
    )
    expect(screen.getByText('What a phone can do')).toBeVisible()
  })

  it('sets what each phone may ask, with your yes by default', async () => {
    const user = userEvent.setup()
    mocks().listOwnDevices = ok(overview({ devices: [device({})] }))
    mocks().setOwnDeviceAskLevel = ok(null)
    render(<DevicesSection />)
    const level = await screen.findByRole('combobox', {
      name: "What Ada's phone may ask",
    })
    expect(level).toHaveValue('confirm')
    await user.selectOptions(level, 'answer')
    await waitFor(() =>
      expect(mocks().setOwnDeviceAskLevel).toHaveBeenCalledWith(
        DEVICE,
        'answer'
      )
    )
  })
})
