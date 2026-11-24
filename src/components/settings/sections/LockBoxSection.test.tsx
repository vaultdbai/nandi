import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { LockBoxOverview } from '@/lib/bindings'
import { LockBoxSection } from './LockBoxSection'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })
const names = [
  'getLockBox',
  'addLockBoxKey',
  'updateLockBoxKey',
  'deleteLockBoxKey',
  'unlockLockBoxKey',
  'lockLockBoxKey',
  'listAgents',
]

const VALUE = 'ghp_never_shown_back_123'

const overview = (patch: Partial<LockBoxOverview> = {}): LockBoxOverview => ({
  keys: [
    {
      name: 'github',
      rules: {
        kind: 'token',
        hosts: ['api.github.com'],
        agents: ['writer'],
        lock: 'locked',
        unlockMinutes: 15,
        header: { name: 'authorization', prefix: 'Bearer ' },
      },
      stored: true,
      createdAt: 1,
      lastUsedAt: null,
      lastUsedBy: null,
      lastUsedHost: null,
      unlockedUntil: null,
    },
  ],
  systemUnlock: 'Windows Hello',
  ...patch,
})

describe('Settings › Lock box', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of names) original[name] = mocked[name]
    mocked.getLockBox = vi.fn(async () => ok(overview()))
    mocked.addLockBoxKey = vi.fn(async () => ok(null))
    mocked.updateLockBoxKey = vi.fn(async () => ok(null))
    mocked.deleteLockBoxKey = vi.fn(async () => ok(null))
    mocked.unlockLockBoxKey = vi.fn(async () => ok(Date.now() + 60_000))
    mocked.lockLockBoxKey = vi.fn(async () => ok(null))
    mocked.listAgents = vi.fn(async () =>
      ok([{ id: 'writer', name: 'Writer' }])
    )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('lists each key by its rules, never a value', async () => {
    render(<LockBoxSection />)
    expect(await screen.findByText('github')).toBeInTheDocument()
    expect(screen.getByText('api.github.com')).toBeInTheDocument()
    expect(await screen.findByText('Writer')).toBeInTheDocument()
    expect(screen.getByText('Locked')).toBeInTheDocument()
    expect(screen.getByText('Not used yet')).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: /Unlock with Windows Hello/ })
    ).toBeInTheDocument()
  })

  it('says when Locked keys ask each time on this computer', async () => {
    mocked.getLockBox = vi.fn(async () => ok(overview({ systemUnlock: null })))
    render(<LockBoxSection />)
    expect(
      await screen.findByText(/Locked keys ask each time instead/)
    ).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: /Unlock with/ })
    ).not.toBeInTheDocument()
  })

  it('adds a key with its value typed once', async () => {
    const user = userEvent.setup()
    mocked.getLockBox = vi.fn(async () => ok(overview({ keys: [] })))
    render(<LockBoxSection />)
    await user.click(await screen.findByRole('button', { name: 'Add a key' }))
    await user.type(screen.getByLabelText('Name'), 'github')
    await user.type(screen.getByLabelText('Value'), VALUE)
    await user.type(screen.getByLabelText('Sites'), 'api.github.com')
    await user.click(await screen.findByRole('checkbox'))
    await user.click(screen.getByRole('button', { name: 'Add the key' }))
    await waitFor(() => expect(mocked.addLockBoxKey).toHaveBeenCalled())
    expect(mocked.addLockBoxKey).toHaveBeenCalledWith({
      name: 'github',
      value: VALUE,
      rules: {
        kind: 'apiKey',
        hosts: ['api.github.com'],
        agents: ['writer'],
        lock: 'ask',
        unlockMinutes: 15,
        header: { name: 'authorization', prefix: 'Bearer ' },
      },
    })
    await waitFor(() =>
      expect(screen.queryByLabelText('Value')).not.toBeInTheDocument()
    )
  })

  it('changes a key without asking for its value again', async () => {
    const user = userEvent.setup()
    render(<LockBoxSection />)
    await user.click(await screen.findByRole('button', { name: 'Edit github' }))
    expect(screen.queryByLabelText('Value')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(mocked.updateLockBoxKey).toHaveBeenCalledWith(
        'github',
        overview().keys[0]?.rules
      )
    )
  })
})
