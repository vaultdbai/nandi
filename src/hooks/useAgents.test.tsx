import { describe, it, expect, vi } from 'vitest'
import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import { commands } from '@/lib/tauri-bindings'
import { useAgents } from './useAgents'

const mockListAgents = vi.mocked(commands.listAgents)

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

describe('useAgents', () => {
  it('returns agents from the backend', async () => {
    const agents = [
      {
        id: 'a',
        name: 'A',
        description: '',
        skillPath: '/a/skill.md',
        status: 'idle' as const,
        currentTask: null,
        origin: { kind: 'local' as const },
        builtin: false,
      },
    ]
    mockListAgents.mockResolvedValueOnce({ status: 'ok', data: agents })

    const { result } = renderHook(() => useAgents(), { wrapper })

    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual(agents)
  })

  it('surfaces backend errors as Error instances', async () => {
    mockListAgents.mockResolvedValueOnce({ status: 'error', error: 'boom' })

    const { result } = renderHook(() => useAgents(), { wrapper })

    await waitFor(() => expect(result.current.isError).toBe(true))
    expect(result.current.error).toBeInstanceOf(Error)
    expect(result.current.error?.message).toBe('boom')
  })
})
