import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { NandiBrowser } from '@/lib/bindings'
import { BrowserSection } from './BrowserSection'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })
const names = [
  'nandiBrowsers',
  'openNandiBrowser',
  'signOutNandiBrowser',
  'listAgents',
]

describe('Settings › Browser', () => {
  const original: Commands = {}
  let browsers: NandiBrowser[]

  beforeEach(() => {
    for (const name of names) original[name] = mocked[name]
    browsers = [
      {
        browser: 'chrome',
        hasProfile: true,
        openInWindow: false,
        inUseBy: null,
        agents: ['cos'],
      },
      {
        browser: 'edge',
        hasProfile: false,
        openInWindow: false,
        inUseBy: 'shopper',
        agents: [],
      },
    ]
    mocked.nandiBrowsers = vi.fn(async () => ok(structuredClone(browsers)))
    mocked.openNandiBrowser = vi.fn(async () => ok(null))
    mocked.signOutNandiBrowser = vi.fn(async () => {
      const chrome = browsers[0]
      if (chrome) chrome.hasProfile = false
      return ok(true)
    })
    mocked.listAgents = vi.fn(async () =>
      ok([
        { id: 'cos', name: 'Chief of Staff' },
        { id: 'shopper', name: 'Shopper' },
      ])
    )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('shows each installed browser, who may use it and the sync note', async () => {
    render(<BrowserSection />)
    const chrome = await screen.findByRole('region', {
      name: 'My Nandi browser (Chrome)',
    })
    expect(
      within(chrome).getByText('Sites remember your sign-ins here')
    ).toBeVisible()
    expect(
      within(chrome).getByText(/Sign in to Chrome in this window/)
    ).toBeVisible()
    expect(
      within(chrome).getByText(/never copies passwords or cookies/)
    ).toBeVisible()
    expect(
      await within(chrome).findByText('Agents that may use it: Chief of Staff')
    ).toBeVisible()
  })

  it('opens the person’s own window with no agent attached', async () => {
    const user = userEvent.setup()
    render(<BrowserSection />)
    const chrome = await screen.findByRole('region', {
      name: 'My Nandi browser (Chrome)',
    })
    await user.click(
      within(chrome).getByRole('button', { name: 'Open my Nandi browser' })
    )
    expect(mocked.openNandiBrowser).toHaveBeenCalledWith('chrome')
  })

  it('says calmly when an agent has it open, and waits for it', async () => {
    render(<BrowserSection />)
    const edge = await screen.findByRole('region', {
      name: 'My Nandi browser (Edge)',
    })
    expect(
      await within(edge).findByText('In use by Shopper right now')
    ).toBeVisible()
    expect(
      within(edge).getByRole('button', { name: 'Open my Nandi browser' })
    ).toBeDisabled()
    expect(
      within(edge).getByRole('button', { name: 'Sign out everywhere' })
    ).toBeDisabled()
  })

  it('signs out everywhere after a confirm', async () => {
    const user = userEvent.setup()
    render(<BrowserSection />)
    const chrome = await screen.findByRole('region', {
      name: 'My Nandi browser (Chrome)',
    })
    await user.click(
      within(chrome).getByRole('button', { name: 'Sign out everywhere' })
    )
    expect(mocked.signOutNandiBrowser).not.toHaveBeenCalled()
    const dialog = await screen.findByRole('alertdialog')
    await user.click(
      within(dialog).getByRole('button', { name: 'Sign out everywhere' })
    )
    expect(mocked.signOutNandiBrowser).toHaveBeenCalledWith('chrome')
    await waitFor(() =>
      expect(
        within(chrome).getByRole('button', { name: 'Sign out everywhere' })
      ).toBeDisabled()
    )
  })

  it('says so when neither Chrome nor Edge is installed', async () => {
    browsers = []
    render(<BrowserSection />)
    expect(
      await screen.findByText(/didn't find Chrome or Edge/)
    ).toBeInTheDocument()
  })
})
