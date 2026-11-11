import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { CapabilityRegistry, SyncResult } from '@/lib/bindings'
import { SourcesSection } from './SourcesSection'

function registry(
  overrides: Partial<CapabilityRegistry> = {}
): CapabilityRegistry {
  return {
    id: 'a1b2c3d4e5f60718',
    url: 'https://github.com/acme/capabilities.git',
    kind: 'github',
    branchPrefix: null,
    userSlug: null,
    lastSyncedAt: Date.now(),
    lastCommit: '0123456789abcdef',
    subscribed: true,
    lastError: null,
    capabilityCount: 3,
    ...overrides,
  }
}

function syncResult(overrides: Partial<SyncResult> = {}): SyncResult {
  return {
    url: 'https://github.com/acme/capabilities.git',
    commit: '0123456789abcdef',
    added: 0,
    updated: 0,
    removed: 0,
    warnings: [],
    error: null,
    ...overrides,
  }
}

// The shared setup doesn't mock the wave A commands; this file does
const mocks = {
  listCapabilityRegistries: vi.fn(),
  addCapabilityRegistry: vi.fn(),
  removeCapabilityRegistry: vi.fn(),
  syncCapabilityRegistry: vi.fn(),
}
Object.assign(commands, mocks)

/** Opens Add source on one kind */
async function openAdd(kind: 'Agents & packs' | 'Capabilities') {
  const [add] = await screen.findAllByRole('button', { name: 'Add source' })
  await userEvent.click(add as HTMLElement)
  await userEvent.click(screen.getByLabelText(kind))
}

describe('Settings › Sources: capabilities', () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) mock.mockReset()
    vi.mocked(commands.listRegistries).mockResolvedValue({
      status: 'ok',
      data: [],
    })
    mocks.listCapabilityRegistries.mockResolvedValue({
      status: 'ok',
      data: [
        registry(),
        registry({
          id: 'ffeeddccbbaa9988',
          url: 'https://git.example.com/broken.git',
          kind: 'generic',
          lastSyncedAt: null,
          lastCommit: null,
          capabilityCount: 0,
          lastError: 'Failed to fetch: host not found',
        }),
      ],
    })
    mocks.addCapabilityRegistry.mockResolvedValue({
      status: 'ok',
      data: registry({ url: 'https://gitlab.com/team/caps.git' }),
    })
    mocks.removeCapabilityRegistry.mockResolvedValue({
      status: 'ok',
      data: null,
    })
    mocks.syncCapabilityRegistry.mockResolvedValue({
      status: 'ok',
      data: syncResult(),
    })
  })

  it('lists registries with count, commit, kind, and errors', async () => {
    render(<SourcesSection />)
    const row = (
      await screen.findByText('https://github.com/acme/capabilities.git')
    ).closest('li') as HTMLElement
    expect(within(row).getByText('GitHub')).toBeInTheDocument()
    expect(within(row).getByText(/Capabilities: 3/)).toBeInTheDocument()
    expect(within(row).getByText('at 0123456')).toBeInTheDocument()
    expect(within(row).queryByRole('alert')).not.toBeInTheDocument()

    const broken = screen
      .getByText('https://git.example.com/broken.git')
      .closest('li') as HTMLElement
    expect(within(broken).getByText(/Never synced/)).toBeInTheDocument()
    expect(within(broken).getByRole('alert')).toHaveTextContent(
      'host not found'
    )
  })

  it('shows an empty state', async () => {
    mocks.listCapabilityRegistries.mockResolvedValue({ status: 'ok', data: [] })
    render(<SourcesSection />)
    expect(
      await screen.findByText(/No sources\. The Library has nothing to browse/)
    ).toBeInTheDocument()
  })

  it('adds a registry, detecting the kind from the URL', async () => {
    render(<SourcesSection />)
    await openAdd('Capabilities')
    const url = await screen.findByLabelText('Git URL or folder')
    await userEvent.type(url, 'https://gitlab.com/team/caps.git')
    expect(
      screen.getByRole('option', { name: 'Detect from URL (GitLab)' })
    ).toBeInTheDocument()
    await userEvent.type(
      screen.getByLabelText('Branch prefix (optional)'),
      'caps'
    )
    await userEvent.click(screen.getByRole('button', { name: 'Add registry' }))
    expect(mocks.addCapabilityRegistry).toHaveBeenCalledWith(
      'https://gitlab.com/team/caps.git',
      null,
      'caps',
      null
    )
    expect(url).toHaveValue('')
  })

  it('reports a failed add', async () => {
    mocks.addCapabilityRegistry.mockResolvedValue({
      status: 'error',
      error: 'Already subscribed to https://gitlab.com/team/caps.git',
    })
    render(<SourcesSection />)
    await openAdd('Capabilities')
    await userEvent.type(
      await screen.findByLabelText('Git URL or folder'),
      'https://gitlab.com/team/caps.git'
    )
    await userEvent.click(screen.getByRole('button', { name: 'Add registry' }))
    const form = screen.getByRole('form', { name: 'Add registry' })
    expect(await within(form).findByRole('alert')).toBeInTheDocument()
    expect(screen.getByLabelText('Git URL or folder')).toHaveValue(
      'https://gitlab.com/team/caps.git'
    )
  })

  it('syncs now and lists the warnings', async () => {
    mocks.syncCapabilityRegistry.mockResolvedValue({
      status: 'ok',
      data: syncResult({
        warnings: ["capabilities/bad.yaml: version 'one' isn't semver"],
      }),
    })
    render(<SourcesSection />)
    await userEvent.click(
      await screen.findByRole('button', {
        name: 'Sync https://github.com/acme/capabilities.git',
      })
    )
    expect(mocks.syncCapabilityRegistry).toHaveBeenCalledWith(
      'https://github.com/acme/capabilities.git'
    )
    expect(
      await screen.findByText(/capabilities\/bad.yaml/)
    ).toBeInTheDocument()
  })

  it('removes a registry after confirming', async () => {
    render(<SourcesSection />)
    await userEvent.click(
      await screen.findByRole('button', {
        name: 'Remove https://github.com/acme/capabilities.git',
      })
    )
    expect(mocks.removeCapabilityRegistry).not.toHaveBeenCalled()
    const dialog = await screen.findByRole('alertdialog')
    await userEvent.click(
      within(dialog).getByRole('button', { name: 'Remove' })
    )
    expect(mocks.removeCapabilityRegistry).toHaveBeenCalledWith(
      'https://github.com/acme/capabilities.git'
    )
  })
})
