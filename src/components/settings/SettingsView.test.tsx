import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { useUIStore } from '@/store/ui-store'
import { SettingsView } from './SettingsView'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

describe('SettingsView', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    useUIStore.setState({ settingsSection: null })
    Object.assign(commands, {
      listCapabilityRegistries: ok([]),
      getDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: null,
      }),
    })
  })

  it('lists the sections in order and opens Models & runtimes by default', () => {
    render(<SettingsView view={{ kind: 'settings' }} />)
    const rail = screen.getByRole('navigation', { name: 'Settings sections' })
    expect(
      within(rail)
        .getAllByRole('button')
        .map(button => button.textContent)
    ).toEqual([
      'Models & runtimes',
      'This computer',
      'Account',
      'Email',
      'Calendar & files',
      'Connector access',
      'Auto-approve',
      'Cloud credentials',
      'Internet access',
      'Backup',
      'Sources',
      'Logs',
      'Appearance',
    ])
    expect(
      within(rail).getByRole('button', { name: 'Models & runtimes' })
    ).toHaveAttribute('aria-current', 'page')
  })

  it('Account › Run setup again reopens the whole setup', async () => {
    const user = userEvent.setup()
    render(<SettingsView view={{ kind: 'settings', section: 'account' }} />)
    await user.click(screen.getByRole('button', { name: 'Run setup again' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'onboarding',
      again: true,
    })
  })

  it('puts Nandi’s model, the coding agents and the keys on one page', () => {
    render(<SettingsView view={{ kind: 'settings', section: 'models' }} />)
    for (const group of [
      'Nandi (built in)',
      'Coding agents',
      'Provider keys',
    ]) {
      expect(screen.getByRole('heading', { name: group })).toBeInTheDocument()
    }
  })

  it('marks Models when no model can be called', async () => {
    render(<SettingsView view={{ kind: 'settings' }} />)
    const rail = screen.getByRole('navigation', { name: 'Settings sections' })
    expect(
      await within(rail).findByRole('img', { name: 'Needs attention' })
    ).toBeInTheDocument()
  })

  it('moves between sections and remembers the last one', async () => {
    const user = userEvent.setup()
    const { unmount } = render(
      <SettingsView view={{ kind: 'settings', section: 'appearance' }} />
    )
    expect(
      screen.getByRole('heading', { name: 'Appearance' })
    ).toBeInTheDocument()
    await user.click(
      within(
        screen.getByRole('navigation', { name: 'Settings sections' })
      ).getByRole('button', { name: 'Logs' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'logs',
    })
    unmount()

    // A plain "Settings" opens on the last section used this session
    render(<SettingsView view={{ kind: 'settings' }} />)
    expect(
      screen.getByRole('heading', { name: 'Appearance' })
    ).toBeInTheDocument()
  })
})
