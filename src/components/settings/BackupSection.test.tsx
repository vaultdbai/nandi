import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { IdentityStatus, RemoteStatus } from '@/lib/bindings'
import { AccountSection } from './sections/AccountSection'
import { BackupSection } from './sections/BackupSection'

/** Account and Backup, as Settings shows them */
function RemoteSettingsView() {
  return (
    <>
      <AccountSection />
      <BackupSection />
    </>
  )
}

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const identity: IdentityStatus = {
  onboarded: true,
  displayName: 'Ada Lovelace',
  userSlug: 'ada',
  branch: 'nandi/ada',
}

const empty: RemoteStatus = {
  configured: false,
  config: null,
  reachable: false,
  targetBranch: 'nandi/ada',
  localHead: '',
  remoteHead: null,
  ahead: 0,
  behind: 0,
  conflicts: false,
  conflictedFiles: [],
}

const configured: RemoteStatus = {
  ...empty,
  configured: true,
  reachable: true,
  config: {
    url: 'https://github.com/ada/nandi-agents.git',
    hasCredentials: true,
    credentialKind: 'token',
    branchPrefix: 'nandi',
    userSlug: 'ada',
    lastPushAt: null,
    lastPullAt: null,
    lastPushCommit: null,
  },
  ahead: 2,
  behind: 1,
}

describe('Settings › Account and Backup', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getIdentityStatus: ok(identity),
      getRemoteStatus: ok(empty),
      configureRemote: ok(configured),
      clearRemote: ok(null),
      testRemoteConnection: ok(null),
      pushToRemote: ok({
        pushedCommits: 2,
        targetBranch: 'nandi/ada',
        remoteHead: 'abc',
        conflict: false,
      }),
      pullFromRemote: ok({
        pulledCommits: 1,
        conflicts: false,
        conflictedFiles: [],
      }),
      listRemoteBranches: ok(['nandi/ada', 'nandi/grace']),
      setDisplayName: ok(null),
      setUserSlug: ok(null),
    })
  })

  it('shows the empty state with the form, and saves a remote', async () => {
    const user = userEvent.setup()
    render(<RemoteSettingsView />)

    expect(await screen.findByText('No remote configured')).toBeInTheDocument()
    await user.type(
      screen.getByLabelText('Repository URL'),
      'https://github.com/ada/agents.git'
    )
    await user.click(
      within(screen.getByTestId('remote-empty')).getByRole('button', {
        name: 'Save',
      })
    )
    await waitFor(() =>
      expect(mocks().configureRemote).toHaveBeenCalledWith(
        'https://github.com/ada/agents.git',
        null,
        'nandi',
        'ada'
      )
    )
  })

  it('shows a configured remote, its branches, and pushes', async () => {
    const user = userEvent.setup()
    Object.assign(commands, { getRemoteStatus: ok(configured) })
    render(<RemoteSettingsView />)

    const card = await screen.findByTestId('remote-status-card')
    expect(card).toHaveTextContent('https://github.com/ada/nandi-agents.git')
    expect(card).toHaveTextContent('Reachable')
    expect(card).toHaveTextContent('nandi/ada')

    const own = (await screen.findByText('Your branch')).closest('li')
    expect(own).toHaveAttribute('data-own', 'true')
    expect(
      within(own as HTMLElement).getByRole('button', {
        name: 'Pull into local',
      })
    ).toBeEnabled()
    const other = screen.getByText('nandi/grace').closest('li') as HTMLElement
    expect(
      within(other).getByRole('button', { name: 'Pull into local' })
    ).toBeDisabled()

    await user.click(screen.getByRole('button', { name: 'Back up now' }))
    // Pushing asks first, naming the remote and branch
    const confirm = await screen.findByRole('alertdialog')
    expect(confirm).toHaveTextContent('nandi/ada')
    expect(mocks().pushToRemote).not.toHaveBeenCalled()
    await user.click(
      within(confirm).getByRole('button', { name: 'Back up now' })
    )
    await waitFor(() =>
      expect(mocks().pushToRemote).toHaveBeenCalledWith(false)
    )
  })

  it('offers overwrite on a push conflict, only for the own branch', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getRemoteStatus: ok(configured),
      pushToRemote: vi
        .fn()
        .mockResolvedValueOnce({
          status: 'ok',
          data: {
            pushedCommits: 0,
            targetBranch: 'nandi/ada',
            remoteHead: 'abc',
            conflict: true,
          },
        })
        .mockResolvedValue({
          status: 'ok',
          data: {
            pushedCommits: 2,
            targetBranch: 'nandi/ada',
            remoteHead: 'def',
            conflict: false,
          },
        }),
    })
    render(<RemoteSettingsView />)

    await user.click(await screen.findByRole('button', { name: 'Back up now' }))
    await user.click(
      within(await screen.findByRole('alertdialog')).getByRole('button', {
        name: 'Back up now',
      })
    )
    const dialog = (
      await screen.findByText(/Only nandi\/ada is touched/)
    ).closest('[role="alertdialog"]') as HTMLElement
    expect(dialog).toBeInTheDocument()
    await user.click(
      within(dialog).getByRole('button', { name: 'Overwrite nandi/ada' })
    )
    await waitFor(() =>
      expect(mocks().pushToRemote).toHaveBeenLastCalledWith(true)
    )
  })

  it('shows the conflict banner with files, a disabled abort, and retry', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getRemoteStatus: ok({
        ...configured,
        conflicts: true,
        conflictedFiles: ['agents/writer/skill.md'],
      }),
    })
    render(<RemoteSettingsView />)

    const banner = await screen.findByTestId('remote-conflict-banner')
    expect(
      within(banner).getByRole('button', { name: 'Abort merge' })
    ).toBeDisabled()
    await user.click(
      within(banner).getByRole('button', { name: 'Show conflicted files' })
    )
    expect(
      within(banner).getByText('agents/writer/skill.md')
    ).toBeInTheDocument()
    await user.click(within(banner).getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(mocks().pullFromRemote).toHaveBeenCalled())
  })

  it('shows conflicts a pull reports', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getRemoteStatus: ok(configured),
      pullFromRemote: ok({
        pulledCommits: 0,
        conflicts: true,
        conflictedFiles: ['a.md', 'b.md'],
      }),
    })
    render(<RemoteSettingsView />)

    await user.click(
      await screen.findByRole('button', { name: 'Restore from remote' })
    )
    expect(
      await screen.findByTestId('remote-conflict-banner')
    ).toHaveTextContent('2 conflicted file(s)')
  })

  it('removes the remote after confirming', async () => {
    const user = userEvent.setup()
    Object.assign(commands, { getRemoteStatus: ok(configured) })
    render(<RemoteSettingsView />)

    await user.click(
      await screen.findByRole('button', { name: 'Remove remote' })
    )
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: 'Remove' }))
    await waitFor(() => expect(mocks().clearRemote).toHaveBeenCalled())
  })

  it('edits the identity', async () => {
    const user = userEvent.setup()
    render(<RemoteSettingsView />)

    const name = await screen.findByLabelText('Your name')
    await user.clear(name)
    await user.type(name, 'Ada King')
    await user.click(
      screen.getAllByRole('button', { name: 'Save' })[0] as HTMLElement
    )
    await waitFor(() =>
      expect(mocks().setDisplayName).toHaveBeenCalledWith('Ada King')
    )
    expect(mocks().setUserSlug).not.toHaveBeenCalled()
  })
})
