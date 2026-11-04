import { render, screen } from '@/test/test-utils'
import { describe, it, expect, vi } from 'vitest'
import { commands } from '@/lib/tauri-bindings'
import App from './App'

// Tauri bindings are mocked globally in src/test/setup.ts (onboarded)

describe('App', () => {
  it('renders the workspace shell on Home when onboarded', async () => {
    render(<App />)
    expect(
      await screen.findByRole('navigation', { name: 'Workspace' })
    ).toBeInTheDocument()
    expect(
      await screen.findByRole('heading', { level: 1, name: 'Home' })
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Account' })).toBeInTheDocument()
    expect(await screen.findByText("Test User's Nandi")).toBeInTheDocument()
  })

  it('shows onboarding when the identity is not set up', async () => {
    // Both the title bar and the gate read the identity
    const getIdentityStatus = vi.mocked(commands.getIdentityStatus)
    const original = getIdentityStatus.getMockImplementation()
    getIdentityStatus.mockResolvedValue({
      status: 'ok',
      data: {
        onboarded: false,
        displayName: null,
        userSlug: null,
        branch: null,
      },
    })
    try {
      render(<App />)
      expect(await screen.findByTestId('onboarding-gate')).toBeInTheDocument()
      expect(
        screen.queryByRole('navigation', { name: 'Workspace' })
      ).not.toBeInTheDocument()
    } finally {
      if (original) getIdentityStatus.mockImplementation(original)
    }
  })

  it('renders title bar with traffic light buttons', () => {
    render(<App />)
    // Find specifically the window control buttons in the title bar
    const titleBarButtons = screen
      .getAllByRole('button')
      .filter(
        button =>
          button.getAttribute('aria-label')?.includes('window') ||
          button.className.includes('window-control')
      )
    // Should have at least the window control buttons
    expect(titleBarButtons.length).toBeGreaterThan(0)
  })
})
