import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { IdentityStatus } from '@/lib/bindings'
import { UserMenu } from '@/components/titlebar/UserMenu'
import { useNavigationStore } from '@/store/navigation-store'
import { logger } from '@/lib/logger'
import { AuthGate } from './AuthGate'

const identity = (overrides: Partial<IdentityStatus> = {}): IdentityStatus => ({
  onboarded: true,
  displayName: 'Mona',
  userSlug: 'mona',
  branch: null,
  onboardingStep: 'done',
  ...overrides,
})

function renderGate() {
  return render(
    <AuthGate>
      <div>app content</div>
    </AuthGate>
  )
}

describe('AuthGate', () => {
  beforeEach(() => {
    vi.mocked(commands.getIdentityStatus).mockReset()
  })

  it('shows the app when onboarded', async () => {
    vi.mocked(commands.getIdentityStatus).mockResolvedValue({
      status: 'ok',
      data: identity(),
    })
    renderGate()
    expect(await screen.findByText('app content')).toBeInTheDocument()
    expect(commands.getIdentityStatus).toHaveBeenCalledTimes(1)
  })

  it('routes to onboarding when not onboarded', async () => {
    vi.mocked(commands.getIdentityStatus).mockResolvedValue({
      status: 'ok',
      data: identity({ onboarded: false, displayName: null, userSlug: null }),
    })
    renderGate()
    expect(await screen.findByTestId('onboarding-gate')).toBeInTheDocument()
    expect(screen.queryByText('app content')).not.toBeInTheDocument()
  })

  it('fresh install routes to onboarding', async () => {
    // No NANDI.md yet: nothing is set
    vi.mocked(commands.getIdentityStatus).mockResolvedValue({
      status: 'ok',
      data: {
        onboarded: false,
        displayName: null,
        userSlug: null,
        branch: null,
        onboardingStep: null,
      },
    })
    renderGate()
    expect(await screen.findByTestId('onboarding-gate')).toBeInTheDocument()
    expect(screen.queryByText('app content')).not.toBeInTheDocument()
  })

  it('keeps onboarding open while a named home has no step recorded', async () => {
    vi.mocked(commands.getIdentityStatus).mockResolvedValue({
      status: 'ok',
      data: identity({ onboardingStep: null }),
    })
    renderGate()
    expect(await screen.findByTestId('onboarding-gate')).toBeInTheDocument()
    expect(screen.queryByText('app content')).not.toBeInTheDocument()
  })

  it('fails open (and logs) when the identity status errors', async () => {
    const warn = vi.spyOn(logger, 'warn').mockImplementation(() => undefined)
    vi.mocked(commands.getIdentityStatus).mockResolvedValue({
      status: 'error',
      error: 'NANDI.md has invalid frontmatter',
    })
    renderGate()
    expect(await screen.findByText('app content')).toBeInTheDocument()
    expect(screen.queryByTestId('onboarding-gate')).not.toBeInTheDocument()
    expect(warn).toHaveBeenCalledWith(
      expect.stringContaining('identity'),
      expect.objectContaining({ error: 'NANDI.md has invalid frontmatter' })
    )
    warn.mockRestore()
  })

  it('fails open when the identity request rejects', async () => {
    const warn = vi.spyOn(logger, 'warn').mockImplementation(() => undefined)
    vi.mocked(commands.getIdentityStatus).mockRejectedValue(
      new Error('transport down')
    )
    renderGate()
    expect(await screen.findByText('app content')).toBeInTheDocument()
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('opens the app when onboarding completes', async () => {
    // Not onboarded at the gate; onboarded by the time the view reads it
    vi.mocked(commands.getIdentityStatus)
      .mockResolvedValueOnce({
        status: 'ok',
        data: identity({ onboarded: false, displayName: null, userSlug: null }),
      })
      .mockResolvedValue({ status: 'ok', data: identity() })
    renderGate()
    expect(await screen.findByText('app content')).toBeInTheDocument()
    expect(screen.queryByTestId('onboarding-gate')).not.toBeInTheDocument()
  })

  it('shows a spinner while loading', () => {
    vi.mocked(commands.getIdentityStatus).mockReturnValue(
      new Promise(() => undefined)
    )
    renderGate()
    expect(screen.queryByText('app content')).not.toBeInTheDocument()
    expect(screen.queryByTestId('onboarding-gate')).not.toBeInTheDocument()
  })
})

describe('UserMenu', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('opens Settings › Account', async () => {
    const user = userEvent.setup()
    render(<UserMenu />)

    await user.click(screen.getByRole('button', { name: 'Account' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Account' }))

    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'account',
    })
  })

  it('opens Settings › Backup', async () => {
    const user = userEvent.setup()
    render(<UserMenu />)

    await user.click(screen.getByRole('button', { name: 'Account' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Backup' }))

    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'backup',
    })
  })
})
