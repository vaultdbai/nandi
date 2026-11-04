import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { GitStatus, RepoStatus } from '@/lib/bindings'
import { LocalCopyPanel } from './LocalCopyPanel'

const status = (overrides: Partial<RepoStatus> = {}): RepoStatus => ({
  root: '/home/me/.nandi/repo',
  agents: 3,
  workflows: 2,
  triggers: 1,
  problems: [],
  watching: true,
  ...overrides,
})

const git: GitStatus = { branch: 'main', ahead: 0, behind: 0, files: [] }

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

describe('LocalCopyPanel', () => {
  beforeEach(() => {
    Object.assign(commands, {
      repoStatus: ok(status()),
      repoGitStatus: ok(git),
    })
  })

  it('shows the local repo path, counts, and git branch', async () => {
    render(<LocalCopyPanel />)

    const card = within(await screen.findByTestId('repo-setup-current'))
    expect(card.getByText('/home/me/.nandi/repo')).toBeInTheDocument()
    expect(
      card.getByText('3 agents · 2 workflows · 1 triggers')
    ).toBeInTheDocument()
    expect(await card.findByTestId('repo-setup-git')).toHaveTextContent('main')
    expect(screen.queryByText(/github/i)).not.toBeInTheDocument()
  })

  it('reports problems in the repo files', async () => {
    Object.assign(commands, {
      repoStatus: ok(
        status({
          problems: [
            {
              kind: 'agent',
              id: 'broken',
              sourcePath: 'agents/broken.yaml',
              error: 'bad yaml',
              isError: true,
            },
          ],
        })
      ),
    })
    render(<LocalCopyPanel />)

    expect(
      await screen.findByText('1 problems in the repo files (1 errors)')
    ).toBeInTheDocument()
  })

  it('shows why the status could not be read', async () => {
    Object.assign(commands, {
      repoStatus: vi
        .fn()
        .mockResolvedValue({ status: 'error', error: 'repo missing' }),
    })
    render(<LocalCopyPanel />)

    expect(await screen.findByRole('alert')).toHaveTextContent('repo missing')
  })
})
