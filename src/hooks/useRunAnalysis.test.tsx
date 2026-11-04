import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import type { AnalysisResponse } from '@/lib/bindings'
import { analysisErrorMessage, useRunAnalysis } from './useRunAnalysis'

const mocks = vi.hoisted(() => ({
  summarizeRun: vi.fn(),
  analyzeRun: vi.fn(),
}))

vi.mock('@/lib/client', () => ({ client: mocks }))

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

const response = (question: string): AnalysisResponse => ({
  summary: `About ${question}`,
  keyRisks: [],
  sources: [],
  question,
  createdAt: 0,
  cached: false,
})

describe('useRunAnalysis', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.summarizeRun.mockResolvedValue({ status: 'ok', data: 'Summary' })
    mocks.analyzeRun.mockImplementation((_runId: string, q: string) =>
      Promise.resolve({ status: 'ok', data: response(q) })
    )
  })

  it('loads the summary and collects answers, latest per question', async () => {
    const { result } = renderHook(() => useRunAnalysis('r1'), { wrapper })
    await waitFor(() => expect(result.current.summary.data).toBe('Summary'))
    expect(result.current.answers).toEqual([])

    await act(() => result.current.ask.mutateAsync('Why?'))
    await act(() => result.current.ask.mutateAsync('How?'))
    await act(() => result.current.ask.mutateAsync('Why?'))
    await waitFor(() =>
      expect(result.current.answers.map(a => a.question)).toEqual([
        'How?',
        'Why?',
      ])
    )
    expect(mocks.analyzeRun).toHaveBeenCalledWith('r1', 'Why?')
  })

  it('does not summarize while disabled', () => {
    renderHook(() => useRunAnalysis('r1', { enabled: false }), { wrapper })
    expect(mocks.summarizeRun).not.toHaveBeenCalled()
  })

  it('surfaces backend errors as text', async () => {
    mocks.summarizeRun.mockResolvedValue({ status: 'error', error: 'No key' })
    const { result } = renderHook(() => useRunAnalysis('r1'), { wrapper })
    await waitFor(() => expect(result.current.summary.error).toBeTruthy())
    expect(analysisErrorMessage(result.current.summary.error)).toBe('No key')
    expect(analysisErrorMessage(new Error('boom'))).toBe('boom')
  })
})
