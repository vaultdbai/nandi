import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { useRunAnalysis } from '@/hooks/useRunAnalysis'
import type { SourceRef } from '@/lib/bindings'
import { RunAskBox, RunSummaryTab } from './RunSummary'
import { formatCitations } from './run-detail-model'

const mocks = vi.hoisted(() => ({
  summarizeRun: vi.fn(),
  analyzeRun: vi.fn(),
}))

vi.mock('@/lib/client', () => ({ client: mocks }))

describe('formatCitations', () => {
  it('turns citation groups into inline code and leaves links alone', () => {
    expect(
      formatCitations('Done [event:1, message:m2]. See [docs](https://x.y).')
    ).toBe('Done `event:1, message:m2`. See [docs](https://x.y).')
  })
})

/** The run page's wiring: the Summary tab plus the ask box under it */
function RunAnalysis({
  runId,
  onOpenSource,
  onAsk = () => undefined,
}: {
  runId: string
  onOpenSource?: (source: SourceRef) => void
  onAsk?: () => void
}) {
  const analysis = useRunAnalysis(runId)
  return (
    <>
      <RunSummaryTab analysis={analysis} onOpenSource={onOpenSource} />
      <RunAskBox ask={analysis.ask} onAsk={onAsk} />
    </>
  )
}

describe('RunSummaryTab and RunAskBox', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.summarizeRun.mockResolvedValue({ status: 'ok', data: 'All fine.' })
  })

  it('asks with Enter and shows the answer, risks, and sources', async () => {
    const user = userEvent.setup()
    const onOpenSource = vi.fn()
    mocks.analyzeRun.mockResolvedValue({
      status: 'ok',
      data: {
        summary: 'It failed once [event:3].',
        keyRisks: ['Retry loop'],
        sources: [{ kind: 'event', id: '3', label: 'Event #3', url: null }],
        question: 'Any errors?',
        createdAt: 0,
        cached: true,
      },
    })
    const onAsk = vi.fn()
    render(<RunAnalysis runId="r1" onOpenSource={onOpenSource} onAsk={onAsk} />)
    expect(await screen.findByText('All fine.')).toBeInTheDocument()

    const box = screen.getByRole('textbox')
    await user.type(box, 'Any errors?{Enter}')
    expect(onAsk).toHaveBeenCalledOnce()
    expect(await screen.findByText('Retry loop')).toBeInTheDocument()
    expect(
      screen.getByText('event:3', { selector: 'p code' })
    ).toBeInTheDocument()
    expect(screen.getByText('Saved answer')).toBeInTheDocument()
    expect(box).toHaveValue('')

    const sources = screen.getByRole('region', { name: 'Sources' })
    await user.click(within(sources).getByText('Event #3'))
    expect(onOpenSource).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'event', id: '3' })
    )
  })

  it('shows question errors and keeps the text', async () => {
    const user = userEvent.setup()
    mocks.analyzeRun.mockResolvedValue({
      status: 'error',
      error: 'Rate limited',
    })
    render(<RunAnalysis runId="r1" />)
    await user.type(screen.getByRole('textbox'), 'Why?')
    await user.click(screen.getByRole('button', { name: 'Send' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Rate limited')
    expect(screen.getByRole('textbox')).toHaveValue('Why?')
  })
})
