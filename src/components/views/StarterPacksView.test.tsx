import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, within } from '@/test/test-utils'
import { packManifest } from '@/test/fixtures'
import { commands } from '@/lib/tauri-bindings'
import type { InstalledPackInfo } from '@/lib/bindings'
import { handleNandiEvent } from '@/lib/agent-sync'
import { useMarketplaceStore } from '@/store/marketplace-store'
import { useNavigationStore } from '@/store/navigation-store'
import { StarterPacksView } from './StarterPacksView'

const research = packManifest('research-pack', 'Research Pack')
const engineering = packManifest('engineering-pack', 'Engineering Pack', {
  capabilities: {
    tools: ['read_file'],
    network: [],
    pilots: [],
    requiresApproval: [],
    aws: [],
    connectors: [],
  },
})

function installed(id: string): InstalledPackInfo {
  return { id, version: '1.0.0', commit: 'abc', agents: [], workflows: [] }
}

describe('StarterPacksView', () => {
  beforeEach(() => {
    useNavigationStore.setState({
      current: { kind: 'starter-packs' },
      history: [],
    })
    useMarketplaceStore.setState({ progress: {} })
    vi.mocked(commands.installPack).mockClear()
    vi.mocked(commands.listMarketplacePacks).mockResolvedValue({
      status: 'ok',
      data: [research, engineering],
    })
  })

  it('shows each pack with what it contains and needs', async () => {
    render(<StarterPacksView />)
    const card = (await screen.findByText('Research Pack')).closest(
      '[data-slot="card"]'
    ) as HTMLElement
    expect(within(card).getByText('shell')).toBeInTheDocument()
    expect(
      within(card).getByText('network: export.arxiv.org')
    ).toBeInTheDocument()
    expect(within(card).getByText('no pilots')).toBeInTheDocument()
    expect(within(card).getByText(/Agents: 1 · 24 kB/)).toBeInTheDocument()

    await userEvent.click(within(card).getByText('Show contents'))
    expect(within(card).getByText('Research Pack Agent')).toBeInTheDocument()

    const other = screen
      .getByText('Engineering Pack')
      .closest('[data-slot="card"]') as HTMLElement
    expect(within(other).getByText('no network')).toBeInTheDocument()
  })

  it('installs the selected packs in order, then opens the dashboard', async () => {
    vi.mocked(commands.installPack)
      .mockResolvedValueOnce({ status: 'ok', data: installed('research-pack') })
      .mockResolvedValueOnce({
        status: 'ok',
        data: installed('engineering-pack'),
      })
    render(<StarterPacksView />)
    const install = await screen.findByRole('button', {
      name: 'Install 0 packs',
    })
    expect(install).toBeDisabled()

    await userEvent.click(screen.getByLabelText('Select Engineering Pack'))
    await userEvent.click(screen.getByLabelText('Select Research Pack'))
    await userEvent.click(
      screen.getByRole('button', { name: 'Install 2 packs' })
    )

    expect(vi.mocked(commands.installPack).mock.calls).toEqual([
      ['engineering-pack'],
      ['research-pack'],
    ])
    // Home welcomes what was just installed
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'home',
      welcome: { packIds: ['engineering-pack', 'research-pack'] },
    })
  })

  it('starts with the official packs ticked, and the user can untick them', async () => {
    const official = packManifest('dev-pack', 'Nandi Dev Pack', {
      tags: ['official', 'engineering'],
    })
    vi.mocked(commands.listMarketplacePacks).mockResolvedValue({
      status: 'ok',
      data: [official, research],
    })
    render(<StarterPacksView />)
    await screen.findByRole('button', { name: 'Install 1 pack' })
    expect(screen.getByLabelText('Select Nandi Dev Pack')).toBeChecked()
    expect(screen.getByLabelText('Select Research Pack')).not.toBeChecked()

    await userEvent.click(screen.getByLabelText('Select Nandi Dev Pack'))
    expect(
      screen.getByRole('button', { name: 'Install 0 packs' })
    ).toBeDisabled()
  })

  it('shows download progress and failures, then lets the user continue', async () => {
    let finish: (
      value: Awaited<ReturnType<typeof commands.installPack>>
    ) => void = () => undefined
    vi.mocked(commands.installPack).mockReturnValueOnce(
      new Promise(resolve => {
        finish = resolve
      })
    )
    render(<StarterPacksView />)
    await userEvent.click(await screen.findByLabelText('Select Research Pack'))
    await userEvent.click(
      screen.getByRole('button', { name: 'Install 1 pack' })
    )

    act(() =>
      handleNandiEvent({
        event: 'marketplace://progress',
        payload: {
          kind: 'pack',
          id: 'research-pack',
          receivedObjects: 5,
          totalObjects: 10,
        },
      })
    )
    expect(await screen.findByText('Downloading… 50%')).toBeInTheDocument()

    await act(async () =>
      finish({ status: 'error', error: 'agent "citer" already exists' })
    )
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'agent "citer" already exists'
    )
    expect(useNavigationStore.getState().current.kind).toBe('starter-packs')
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument()
    await userEvent.click(
      screen.getByRole('button', { name: 'Continue without them' })
    )
    // Nothing installed: no welcome to show
    expect(useNavigationStore.getState().current).toEqual({ kind: 'home' })
  })

  it('retries a pack that failed', async () => {
    vi.mocked(commands.installPack)
      .mockResolvedValueOnce({ status: 'error', error: 'network down' })
      .mockResolvedValueOnce({ status: 'ok', data: installed('research-pack') })
    render(<StarterPacksView />)
    await userEvent.click(await screen.findByLabelText('Select Research Pack'))
    await userEvent.click(
      screen.getByRole('button', { name: 'Install 1 pack' })
    )
    await userEvent.click(await screen.findByRole('button', { name: 'Retry' }))

    expect(vi.mocked(commands.installPack)).toHaveBeenCalledTimes(2)
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'home',
      welcome: { packIds: ['research-pack'] },
    })
  })

  it('skips to the dashboard', async () => {
    render(<StarterPacksView />)
    await userEvent.click(await screen.findByRole('button', { name: 'Skip' }))
    expect(useNavigationStore.getState().current.kind).toBe('home')
    expect(commands.installPack).not.toHaveBeenCalled()
  })

  it("hides installed packs and won't select ones that need a newer Nandi", async () => {
    vi.mocked(commands.listMarketplacePacks).mockResolvedValue({
      status: 'ok',
      data: [
        { ...research, installedVersion: '1.0.0' },
        { ...engineering, compatible: false, minNandiVersion: '9.0.0' },
      ],
    })
    render(<StarterPacksView />)
    expect(
      await screen.findByText('Needs Nandi 9.0.0 or newer')
    ).toBeInTheDocument()
    expect(screen.queryByText('Research Pack')).not.toBeInTheDocument()
    expect(screen.getByLabelText('Select Engineering Pack')).toBeDisabled()
  })

  it('offers a retry or skip when the registry is unreachable', async () => {
    vi.mocked(commands.listMarketplacePacks).mockResolvedValue({
      status: 'error',
      error: 'Couldn’t reach github.com/vaultdbai/nandi-registry',
    })
    render(<StarterPacksView />)
    expect(
      await screen.findByText("Couldn't load the starter packs")
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Skip' })).toBeInTheDocument()
  })
})
