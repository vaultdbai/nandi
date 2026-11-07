import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { ToolsStatus } from '@/lib/bindings'
import { emitMockEvent } from '@/test/mock-events'
import { ClaudeCodeToolsPanel } from './ClaudeCodeToolsPanel'

const status = (overrides: Partial<ToolsStatus> = {}): ToolsStatus => ({
  volume: 'nandi-tools-claude-code-abc',
  ready: false,
  installing: false,
  dockerRunning: true,
  downloadBytes: 95 * 1024 * 1024,
  log: [],
  error: null,
  ...overrides,
})

const mock = (data: ToolsStatus) =>
  Object.assign(commands, {
    getToolsStatus: vi.fn().mockResolvedValue({ status: 'ok', data }),
    installTools: vi.fn().mockResolvedValue({ status: 'ok', data: null }),
  })

describe('ClaudeCodeToolsPanel', () => {
  it('says Claude Code is fetched on first use, and can get it now', async () => {
    mock(status())
    const user = userEvent.setup()
    render(<ClaudeCodeToolsPanel />)
    expect(await screen.findByTestId('tools-state')).toHaveTextContent(
      'Fetched the first time an agent uses it (one time, about 95 MB).'
    )
    await user.click(screen.getByRole('button', { name: 'Get it now' }))
    expect(commands.installTools).toHaveBeenCalledWith('claudeCode')
    mock(status({ installing: true }))
    act(() =>
      emitMockEvent('images://tools', {
        bundle: 'claudeCode',
        downloadBytes: 95 * 1024 * 1024,
        line: 'added 116 packages',
        finished: null,
        error: null,
      })
    )
    expect(await screen.findByText(/added 116 packages/)).toBeInTheDocument()
    expect(screen.getByTestId('tools-state')).toHaveTextContent(
      'Getting Claude Code (one time, about 95 MB)…'
    )
  })

  it('offers Try again after a failure', async () => {
    mock(status({ error: "Couldn't get Claude Code (ENOTFOUND)" }))
    const user = userEvent.setup()
    render(<ClaudeCodeToolsPanel />)
    expect(await screen.findByTestId('tools-state')).toHaveTextContent(
      'ENOTFOUND'
    )
    await user.click(screen.getByRole('button', { name: 'Try again' }))
    expect(commands.installTools).toHaveBeenCalledWith('claudeCode')
  })

  it('is ready with no button once installed', async () => {
    mock(status({ ready: true }))
    render(<ClaudeCodeToolsPanel />)
    expect(await screen.findByTestId('tools-state')).toHaveTextContent(
      'Ready in the sandbox.'
    )
    expect(screen.queryByRole('button')).toBeNull()
  })
})
