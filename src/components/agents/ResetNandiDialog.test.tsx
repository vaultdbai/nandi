import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { commands } from '@/lib/tauri-bindings'
import { render, screen, waitFor } from '@/test/test-utils'
import { ResetNandiDialog } from './ResetNandiDialog'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

describe('ResetNandiDialog', () => {
  const original: Commands = {}
  let reset: ReturnType<typeof vi.fn>

  beforeEach(() => {
    original.resetDefaultAgent = mocked.resetDefaultAgent
    reset = vi.fn().mockResolvedValue(ok(null))
    mocked.resetDefaultAgent = reset
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  const renderDialog = (onOpenChange = vi.fn()) => {
    render(
      <ResetNandiDialog
        agent={{ name: 'Nandi' }}
        open
        onOpenChange={onOpenChange}
      />
    )
    return onOpenChange
  }

  it('names what changes, what stays and where the old versions go', () => {
    renderDialog()
    expect(
      screen.getByRole('heading', { name: 'Reset Nandi to defaults?' })
    ).toBeInTheDocument()
    const changes = screen.getAllByRole('listitem').map(li => li.textContent)
    expect(changes).toEqual([
      'Its settings: name, model, network and tools (agent.md)',
      'Your rules, back to the template (instructions.md)',
      'Its playbooks: edited ones go back, ones you added are removed',
      'Its morning digest and mail check, even if you removed them',
    ])
    expect(screen.getByText(/memory, runs, chats/)).toBeInTheDocument()
    expect(screen.getByText(/git history/)).toBeInTheDocument()
  })

  it('resets nothing until the confirm', async () => {
    const user = userEvent.setup()
    const onOpenChange = renderDialog()
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(reset).not.toHaveBeenCalled()
  })

  it('resets Nandi when confirmed', async () => {
    const user = userEvent.setup()
    const onOpenChange = renderDialog()
    await user.click(screen.getByRole('button', { name: 'Reset to defaults' }))
    await waitFor(() => expect(reset).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  })
})
