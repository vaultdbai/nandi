import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { RegistryInfo } from '@/lib/bindings'
import { SourcesSection } from './SourcesSection'

function registry(overrides: Partial<RegistryInfo> = {}): RegistryInfo {
  return {
    url: 'github.com/vaultdbai/nandi-registry',
    kind: 'public',
    hasAuth: false,
    isDefault: true,
    agentCount: 16,
    packCount: 5,
    fetchedAt: new Date().toISOString(),
    warning: null,
    ...overrides,
  }
}

/** Opens Add source on one kind */
async function openAdd(kind: 'Agents & packs' | 'Capabilities') {
  const [add] = await screen.findAllByRole('button', { name: 'Add source' })
  await userEvent.click(add as HTMLElement)
  await userEvent.click(screen.getByLabelText(kind))
}

describe('Settings › Sources: agents & packs', () => {
  beforeEach(() => {
    for (const command of [
      commands.addRegistry,
      commands.removeRegistry,
      commands.createCommunityRegistry,
    ]) {
      vi.mocked(command).mockClear()
    }
    Object.assign(commands, {
      listCapabilityRegistries: vi
        .fn()
        .mockResolvedValue({ status: 'ok', data: [] }),
    })
    vi.mocked(commands.listRegistries).mockResolvedValue({
      status: 'ok',
      data: [
        registry(),
        registry({
          url: 'github.com/acme/nandi-community-acme',
          kind: 'community',
          hasAuth: true,
          isDefault: false,
          agentCount: null,
          packCount: null,
          fetchedAt: null,
          warning: 'github.com/acme/nandi-community-acme refused access',
        }),
      ],
    })
  })

  it('lists registries with counts, kind, and problems', async () => {
    render(<SourcesSection />)
    const publicRow = (
      await screen.findByText('github.com/vaultdbai/nandi-registry')
    ).closest('li') as HTMLElement
    expect(within(publicRow).getByText('Default')).toBeInTheDocument()
    expect(
      within(publicRow).getByText(/Agents: 16 · Packs: 5/)
    ).toBeInTheDocument()
    const community = screen
      .getByText('github.com/acme/nandi-community-acme')
      .closest('li') as HTMLElement
    expect(within(community).getByText('Private')).toBeInTheDocument()
    expect(within(community).getByRole('alert')).toHaveTextContent(
      'refused access'
    )
  })

  it('adds a private registry with the GitHub sign-in', async () => {
    vi.mocked(commands.addRegistry).mockResolvedValueOnce({
      status: 'ok',
      data: registry({ url: 'github.com/team/reg', isDefault: false }),
    })
    render(<SourcesSection />)
    await openAdd('Agents & packs')
    await userEvent.type(
      await screen.findByLabelText('URL'),
      'github.com/team/reg'
    )
    await userEvent.click(
      screen.getByLabelText('Use my GitHub sign-in (private repos)')
    )
    expect(
      screen.queryByLabelText('GitHub token (optional)')
    ).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Add registry' }))
    expect(commands.addRegistry).toHaveBeenCalledWith({
      url: 'github.com/team/reg',
      kind: 'community',
      token: null,
      useGithubSignIn: true,
    })
  })

  it('shows why a registry could not be added', async () => {
    vi.mocked(commands.addRegistry).mockResolvedValueOnce({
      status: 'error',
      error: 'github.com/x/y wasn’t found',
    })
    render(<SourcesSection />)
    await openAdd('Agents & packs')
    await userEvent.type(await screen.findByLabelText('URL'), 'github.com/x/y')
    await userEvent.type(
      screen.getByLabelText('GitHub token (optional)'),
      'ghp_x'
    )
    await userEvent.click(screen.getByRole('button', { name: 'Add registry' }))
    // In plain words; the raw text is behind Details
    expect(
      await screen.findByText('It wasn’t found. Check the name or address.')
    ).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Details' }))
    expect(
      screen.getByText(/github.com\/x\/y wasn’t found/)
    ).toBeInTheDocument()
    expect(vi.mocked(commands.addRegistry).mock.calls[0]?.[0].token).toBe(
      'ghp_x'
    )
  })

  it('removes a registry after confirmation', async () => {
    render(<SourcesSection />)
    await userEvent.click(
      await screen.findByRole('button', {
        name: 'Remove github.com/acme/nandi-community-acme',
      })
    )
    const confirm = await screen.findByRole('alertdialog')
    await userEvent.click(
      within(confirm).getByRole('button', { name: 'Remove' })
    )
    expect(commands.removeRegistry).toHaveBeenCalledWith(
      'github.com/acme/nandi-community-acme'
    )
  })

  it('creates a community registry and shows what to share', async () => {
    vi.mocked(commands.createCommunityRegistry).mockResolvedValueOnce({
      status: 'ok',
      data: {
        registry: registry({
          url: 'github.com/alice/nandi-community-alice',
          kind: 'community',
          isDefault: false,
          hasAuth: true,
        }),
        inviteUrl:
          'https://github.com/alice/nandi-community-alice/settings/access',
        created: true,
      },
    })
    render(<SourcesSection />)
    await openAdd('Agents & packs')
    await userEvent.click(
      await screen.findByRole('button', {
        name: 'Create my community registry',
      })
    )
    expect(
      await screen.findByText('Created and added to your registries.')
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        'https://github.com/alice/nandi-community-alice/settings/access'
      )
    ).toBeInTheDocument()
  })
})
