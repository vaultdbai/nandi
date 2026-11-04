import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor } from '@/test/test-utils'
import { emitMockEvent } from '@/test/mock-events'
import { commands } from '@/lib/tauri-bindings'
import type { AgentInfo } from '@/lib/bindings'
import { PilotConsentPrompt } from './PilotConsentPrompt'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const needed = () =>
  act(() =>
    emitMockEvent('pilot://consent_needed', {
      agentId: 'writer',
      pilotId: 'oswright',
      capabilities: ['app_control', 'browser'],
    })
  )

describe('PilotConsentPrompt', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of ['listPilots', 'grantPilotCapability'])
      original[name] = mocked[name]
    mocked.listPilots = vi.fn().mockResolvedValue(
      ok([
        {
          id: 'oswright',
          name: 'OSWright',
          platform: 'any',
          installCommand: '',
          startCommand: '',
          mcpPort: 17898,
          description: '',
          capabilities: ['app_control', 'browser'],
        },
      ])
    )
    mocked.grantPilotCapability = vi.fn().mockResolvedValue(ok(null))
    vi.mocked(commands.listAgents).mockResolvedValue(
      ok([{ id: 'writer', name: 'Writer' } as AgentInfo])
    )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('asks for consent when a run needs it, and grants every capability', async () => {
    const user = userEvent.setup()
    render(<PilotConsentPrompt />)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()

    await needed()
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent(/Writer/)
    expect(dialog).toHaveTextContent(/OSWright/)

    for (const checkbox of screen.getAllByRole('checkbox')) {
      await user.click(checkbox)
    }
    await user.click(screen.getByRole('button', { name: /allow/i }))
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    )
    expect(mocked.grantPilotCapability).toHaveBeenCalledWith(
      'writer',
      'oswright',
      'app_control'
    )
    expect(mocked.grantPilotCapability).toHaveBeenCalledWith(
      'writer',
      'oswright',
      'browser'
    )

    // A grant isn't a dismissal: a later need (e.g. after a revoke) asks again
    await needed()
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
  })

  it('stays quiet for the session once dismissed', async () => {
    const user = userEvent.setup()
    render(<PilotConsentPrompt />)
    await needed()
    await screen.findByRole('dialog')

    await user.keyboard('{Escape}')
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    )

    await needed()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(mocked.grantPilotCapability).not.toHaveBeenCalled()
  })
})
