import { describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { commands } from '@/lib/tauri-bindings'
import { emitMockEvent } from '@/test/mock-events'
import { useShareAgent, useTeams } from './useTeams'

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

describe('useTeams', () => {
  it('lists teams and refetches on team://changed', async () => {
    const listTeams = vi.fn().mockResolvedValue({ status: 'ok', data: [] })
    Object.assign(commands, { listTeams })

    const { result } = renderHook(() => useTeams(), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(listTeams).toHaveBeenCalledTimes(1)

    act(() => emitMockEvent('team://changed', { teamId: 'acme' }))
    await waitFor(() => expect(listTeams).toHaveBeenCalledTimes(2))
  })

  it('surfaces backend errors as Error instances', async () => {
    Object.assign(commands, {
      listTeams: vi
        .fn()
        .mockResolvedValue({ status: 'error', error: 'Not signed in' }),
    })
    const { result } = renderHook(() => useTeams(), { wrapper })
    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(result.current.error).toBeInstanceOf(Error)
    expect(result.current.error?.message).toBe('Not signed in')
  })
})

describe('useShareAgent', () => {
  it('shares and unshares through the core', async () => {
    const shareAgent = vi.fn().mockResolvedValue({ status: 'ok', data: null })
    const unshareAgent = vi.fn().mockResolvedValue({
      status: 'error',
      error: 'shared by hubot',
    })
    Object.assign(commands, { shareAgent, unshareAgent })

    const { result } = renderHook(() => useShareAgent(), { wrapper })
    await act(() =>
      result.current.mutateAsync({
        teamId: 'acme',
        agentId: 'papers',
        share: true,
      })
    )
    expect(shareAgent).toHaveBeenCalledWith('acme', 'papers')

    await expect(
      result.current.mutateAsync({
        teamId: 'acme',
        agentId: 'writer',
        share: false,
      })
    ).rejects.toThrow('shared by hubot')
    expect(unshareAgent).toHaveBeenCalledWith('acme', 'writer')
  })
})
