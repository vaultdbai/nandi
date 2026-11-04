import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { agentInfo, agentManifest } from '@/test/fixtures'
import { commands } from '@/lib/tauri-bindings'
import type { RegistryInfo } from '@/lib/bindings'
import { PublishDialog } from './PublishDialog'

function registry(url: string, kind: string): RegistryInfo {
  return {
    url,
    kind,
    hasAuth: kind === 'community',
    isDefault: kind === 'public',
    agentCount: 1,
    packCount: 0,
    fetchedAt: null,
    warning: null,
  }
}

const writer = agentInfo('writer', 'Writer')

describe('PublishDialog', () => {
  beforeEach(() => {
    vi.mocked(commands.publishAgent).mockClear()
    vi.mocked(commands.listRegistries).mockResolvedValue({
      status: 'ok',
      data: [
        registry('github.com/vaultdbai/nandi-registry', 'public'),
        registry('github.com/alice/nandi-community-alice', 'community'),
        registry('file:///tmp/local', 'personal'),
      ],
    })
    vi.mocked(commands.listAvailableAgents).mockResolvedValue({
      status: 'ok',
      data: [agentManifest('writer', 'Writer', { latestVersion: '1.2.0' })],
    })
  })

  it('prefers a community registry and bumps the listed version', async () => {
    vi.mocked(commands.publishAgent).mockResolvedValueOnce({
      status: 'ok',
      data: {
        prUrl: 'https://github.com/alice/nandi-community-alice/pull/3',
        merged: true,
        tag: 'writer-v1.2.1',
        version: '1.2.1',
        signed: false,
      },
    })
    render(<PublishDialog agent={writer} open onOpenChange={() => undefined} />)
    const select = await screen.findByLabelText('Registry')
    expect(select).toHaveValue('github.com/alice/nandi-community-alice')
    // Only GitHub registries can take a pull request
    expect(screen.queryByText('file:///tmp/local')).not.toBeInTheDocument()
    expect(
      await screen.findByText('Currently listed: v1.2.0')
    ).toBeInTheDocument()
    expect(screen.getByLabelText('Version')).toHaveValue('1.2.1')

    await userEvent.type(
      screen.getByLabelText('Tags (optional)'),
      'writing, docs'
    )
    await userEvent.click(screen.getByRole('button', { name: 'Publish' }))
    expect(commands.publishAgent).toHaveBeenCalledWith({
      agentId: 'writer',
      registryUrl: 'github.com/alice/nandi-community-alice',
      version: '1.2.1',
      tags: ['writing', 'docs'],
    })
    expect(await screen.findByText('Writer is published')).toBeInTheDocument()
    expect(screen.getByText(/writer-v1.2.1/)).toBeInTheDocument()
  })

  it('explains why publishing failed, e.g. changes not backed up', async () => {
    vi.mocked(commands.publishAgent).mockResolvedValueOnce({
      status: 'error',
      error:
        'The agent has changes that aren’t backed up yet. Use Back up now in Settings › Backup, then publish.',
    })
    render(<PublishDialog agent={writer} open onOpenChange={() => undefined} />)
    await screen.findByLabelText('Registry')
    await userEvent.click(screen.getByRole('button', { name: 'Publish' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Back up now in Settings › Backup'
    )
  })

  it('rejects a version that is not x.y.z', async () => {
    render(<PublishDialog agent={writer} open onOpenChange={() => undefined} />)
    const version = await screen.findByLabelText('Version')
    await userEvent.clear(version)
    await userEvent.type(version, 'one')
    expect(screen.getByRole('button', { name: 'Publish' })).toBeDisabled()
  })
})
