import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { HarnessSessionSummary } from '@/lib/bindings'
import { HarnessSessionList } from './HarnessSessionList'

function session(
  id: string,
  overrides: Partial<HarnessSessionSummary> = {}
): HarnessSessionSummary {
  return {
    id,
    adapterId: 'claude-code',
    agentId: 'writer',
    runId: 'r1',
    status: 'finished',
    childPid: 42,
    containerName: null,
    workingDir: '/agent/workdir',
    model: 'claude-sonnet-5',
    provider: 'bedrock',
    stopReason: 'end_turn',
    error: null,
    inputTokens: 10,
    outputTokens: 5,
    startedAt: Date.now(),
    endedAt: Date.now(),
    ...overrides,
  }
}

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

describe('HarnessSessionList', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listHarnessSessions: ok([
        session('s1', { status: 'running', endedAt: null }),
        session('s2', { status: 'failed', error: 'CLI crashed' }),
      ]),
      killHarnessSession: ok(null),
    })
  })

  it('lists sessions with their status and error', async () => {
    render(<HarnessSessionList agentId="writer" />)
    expect(await screen.findAllByTestId('harness-session')).toHaveLength(2)
    expect(screen.getByText('CLI crashed')).toBeInTheDocument()
    expect(commands.listHarnessSessions).toHaveBeenCalledWith('writer', null)
  })

  it('stops only live sessions', async () => {
    const user = userEvent.setup()
    render(<HarnessSessionList />)
    const stop = await screen.findAllByRole('button', { name: /stop/i })
    expect(stop).toHaveLength(1)
    await user.click(stop[0] as HTMLElement)
    await waitFor(() =>
      expect(commands.killHarnessSession).toHaveBeenCalledWith('s1')
    )
  })

  it('says when there are none', async () => {
    Object.assign(commands, { listHarnessSessions: ok([]) })
    render(<HarnessSessionList />)
    expect(
      await screen.findByText(
        /no harness sessions|harness\.sessions\.emptyTitle/i
      )
    ).toBeInTheDocument()
  })
})
