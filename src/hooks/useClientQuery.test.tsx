import { renderHook, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { useClientMutation, useClientQuery } from './useClientQuery'

function setup() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
  return { queryClient, wrapper }
}

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })
const failed = (error: string) => ({ status: 'error' as const, error })

describe('useClientQuery', () => {
  it('resolves to the data of an ok result', async () => {
    const { wrapper } = setup()
    const { result } = renderHook(
      () => useClientQuery(['n'], () => Promise.resolve(ok(7))),
      { wrapper }
    )

    await waitFor(() => expect(result.current.data).toBe(7))
  })

  it('fails with an Error carrying the core error text', async () => {
    const { wrapper } = setup()
    const { result } = renderHook(
      () => useClientQuery(['n'], () => Promise.resolve(failed('no relay'))),
      { wrapper }
    )

    await waitFor(() => expect(result.current.error?.message).toBe('no relay'))
  })
})

describe('useClientMutation', () => {
  it('passes the variables to the call and resolves to its data', async () => {
    const { wrapper } = setup()
    const call = vi.fn((n: number) => Promise.resolve(ok(n * 2)))
    const { result } = renderHook(() => useClientMutation(call), { wrapper })

    await expect(result.current.mutateAsync(4)).resolves.toBe(8)
    expect(call).toHaveBeenCalledWith(4)
  })

  it('fails with an Error carrying the core error text', async () => {
    const { wrapper } = setup()
    const { result } = renderHook(
      () => useClientMutation(() => Promise.resolve(failed('refused'))),
      { wrapper }
    )

    await expect(result.current.mutateAsync()).rejects.toThrow('refused')
  })

  it.each([
    ['worked', () => Promise.resolve(ok(null))],
    ['failed', () => Promise.resolve(failed('refused'))],
  ])('refetches the invalidated key when the call %s', async (_, call) => {
    const { queryClient, wrapper } = setup()
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
    const { result } = renderHook(
      () => useClientMutation(call, { invalidates: ['holds'] }),
      { wrapper }
    )

    await result.current.mutateAsync().catch(() => undefined)

    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['holds'] })
  })

  it('runs the caller onSettled too', async () => {
    const { wrapper } = setup()
    const onSettled = vi.fn()
    const { result } = renderHook(
      () =>
        useClientMutation(() => Promise.resolve(ok(1)), {
          invalidates: ['holds'],
          onSettled,
        }),
      { wrapper }
    )

    await result.current.mutateAsync()

    expect(onSettled).toHaveBeenCalledOnce()
  })
})
