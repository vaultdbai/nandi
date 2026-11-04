import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@/test/test-utils'
import type { RunDetail } from '@/lib/tauri-bindings'
import { run, userMessage } from '@/test/fixtures'
import { RunTimeline } from './RunTimeline'

const mocks = vi.hoisted(() => ({ getRunDetail: vi.fn() }))
vi.mock('@/lib/client', () => ({ client: mocks }))

describe('RunTimeline', () => {
  it('lists steps with status dots, ending in a running pulse', async () => {
    const detail: RunDetail = {
      run: run('r1', 'writer', { status: 'running' }),
      agentName: 'Writer',
      messages: [
        userMessage('u1', 'Go'),
        {
          kind: 'error',
          id: 'e1',
          runId: 'r1',
          createdAt: 5,
          content: 'Disk full',
        },
      ],
      events: [],
    }
    mocks.getRunDetail.mockResolvedValue({ status: 'ok', data: detail })
    render(<RunTimeline runId="r1" />)

    const list = await screen.findByRole('list', { name: 'Timeline' })
    const items = within(list).getAllByRole('listitem')
    expect(items).toHaveLength(4)
    expect(
      within(items[0] as HTMLElement).getByRole('img', { name: 'Done' })
    ).toBeInTheDocument()
    expect(
      within(items[2] as HTMLElement).getByText('Disk full')
    ).toBeInTheDocument()
    expect(
      within(items[2] as HTMLElement).getByRole('img', { name: 'Failed' })
    ).toBeInTheDocument()
    expect(
      within(items[3] as HTMLElement).getByText('Running…')
    ).toBeInTheDocument()
    expect(
      within(items[3] as HTMLElement).getByRole('img', { name: 'Running' })
    ).toBeInTheDocument()
  })
})
