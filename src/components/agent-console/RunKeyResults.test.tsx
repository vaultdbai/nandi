import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import type { RunDetail } from '@/lib/tauri-bindings'
import { agentMessage, run } from '@/test/fixtures'
import { RunKeyResults } from './RunKeyResults'
import { formatBytes } from './run-detail-model'

const mocks = vi.hoisted(() => ({ getRunDetail: vi.fn() }))
vi.mock('@/lib/client', () => ({ client: mocks }))

function serve(output: string) {
  const detail: RunDetail = {
    run: run('r1', 'writer', {
      status: 'completed',
      endedAt: run('x', 'writer').startedAt + 2_000,
      inputTokens: 10,
      outputTokens: 5,
    }),
    agentName: 'Writer',
    messages: [agentMessage('m1', output)],
    events: [],
  }
  mocks.getRunDetail.mockResolvedValue({ status: 'ok', data: detail })
}

describe('RunKeyResults', () => {
  it('shows real stats, and JSON numbers as cards with a chart', async () => {
    serve('```json\n{"positions": 127, "riskScore": 3.2}\n```')
    render(<RunKeyResults runId="r1" />)
    expect(await screen.findByText('Key results')).toBeInTheDocument()
    expect(screen.getByText('Tool calls')).toBeInTheDocument()
    expect(screen.getByText('127')).toBeInTheDocument()
    expect(screen.getAllByText('riskScore').length).toBeGreaterThan(0)
    expect(screen.getByRole('img', { name: 'Values' })).toBeInTheDocument()
    // No cost recorded
    expect(screen.getAllByText('—').length).toBeGreaterThan(0)
  })

  it('falls back to the text and offers the Files tab', async () => {
    const user = userEvent.setup()
    const onViewFiles = vi.fn()
    serve('Nothing numeric here.')
    render(<RunKeyResults runId="r1" onViewFiles={onViewFiles} />)
    expect(await screen.findByText('Nothing numeric here.')).toBeInTheDocument()
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'View all files' }))
    expect(onViewFiles).toHaveBeenCalled()
  })

  it('formats sizes', () => {
    expect(formatBytes(512, 'en')).toBe('512 B')
    expect(formatBytes(2048, 'en')).toBe('2 KB')
    expect(formatBytes(1.5 * 1024 * 1024, 'en')).toBe('1.5 MB')
  })
})
