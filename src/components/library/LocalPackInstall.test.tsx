import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { open } from '@tauri-apps/plugin-dialog'
import { render, screen, waitFor } from '@/test/test-utils'
import { packManifest } from '@/test/fixtures'
import { commands } from '@/lib/tauri-bindings'
import { LocalPackInstall } from './LocalPackInstall'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

const FOLDER = 'D:/packs/offline-pack'
const pack = packManifest('offline-pack', 'Offline Pack', {
  registry: 'local',
  verified: false,
  publisher: '',
})

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

describe('LocalPackInstall', () => {
  beforeEach(() => {
    vi.mocked(open).mockResolvedValue(FOLDER)
    Object.assign(commands, {
      inspectLocalPack: ok(pack),
      installLocalPack: ok({
        id: 'offline-pack',
        version: '1.0.0',
        commit: 'abc',
        agents: [],
        workflows: [],
      }),
      getAgentConsent: ok({ required: false, categories: [] }),
    })
  })

  it('installs a pack from a folder through the same consent dialog, unverified', async () => {
    const user = userEvent.setup()
    render(<LocalPackInstall />)
    await user.click(
      screen.getByRole('button', { name: 'Install from this computer' })
    )
    await user.click(
      await screen.findByRole('menuitem', { name: 'A pack folder…' })
    )

    expect(await screen.findByText(/From this computer/)).toBeInTheDocument()
    expect(commands.inspectLocalPack).toHaveBeenCalledWith(FOLDER)
    // Nothing proves who published it
    expect(screen.getByRole('alert')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Install anyway' }))

    await waitFor(() =>
      expect(commands.installLocalPack).toHaveBeenCalledWith(FOLDER)
    )
  })

  it('does nothing when the picker is cancelled', async () => {
    vi.mocked(open).mockResolvedValue(null)
    const user = userEvent.setup()
    render(<LocalPackInstall />)
    await user.click(
      screen.getByRole('button', { name: 'Install from this computer' })
    )
    await user.click(
      await screen.findByRole('menuitem', { name: 'A pack .zip file…' })
    )
    await waitFor(() => expect(open).toHaveBeenCalled())
    expect(commands.inspectLocalPack).not.toHaveBeenCalled()
  })
})
