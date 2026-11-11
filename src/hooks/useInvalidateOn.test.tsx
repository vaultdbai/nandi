import { renderHook } from '@testing-library/react'
import { QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { NandiEvent, Subscription } from '@/lib/client'
import { queryClient } from '@/lib/query-client'
import { useInvalidateOn } from './useInvalidateOn'

const subscribers: ((event: NandiEvent) => void)[] = []
const unsubscribe = vi.fn()

vi.mock('@/lib/client', () => ({
  client: {
    subscribe: (handler: (event: NandiEvent) => void): Subscription => {
      subscribers.push(handler)
      return { ready: Promise.resolve(), unsubscribe }
    },
  },
}))

const KEY = ['inbox', 'holds'] as const
const NAMES = ['gateway://hold_changed'] as const

function emit(event: NandiEvent['event']) {
  for (const handler of subscribers) {
    handler({ event, payload: {} } as NandiEvent)
  }
}

describe('useInvalidateOn', () => {
  const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )

  beforeEach(() => {
    subscribers.length = 0
    vi.clearAllMocks()
  })

  it.each([
    ['a named event', 'gateway://hold_changed'],
    ['a resync, which no caller has to name', 'nandi://resync'],
  ] as const)('refetches the key on %s', (_, event) => {
    renderHook(() => useInvalidateOn(NAMES, KEY), { wrapper })

    emit(event)

    expect(invalidate).toHaveBeenCalledExactlyOnceWith({ queryKey: KEY })
  })

  it('ignores other events', () => {
    renderHook(() => useInvalidateOn(NAMES, KEY), { wrapper })

    emit('agent://token')

    expect(invalidate).not.toHaveBeenCalled()
  })

  it('does not resubscribe when the key is rebuilt on each render', () => {
    const { rerender } = renderHook(() => useInvalidateOn(NAMES, [...KEY]), {
      wrapper,
    })
    rerender()

    expect(subscribers).toHaveLength(1)
  })

  it('refetches the latest key', () => {
    const { rerender } = renderHook(({ key }) => useInvalidateOn(NAMES, key), {
      wrapper,
      initialProps: { key: ['thread', 'a'] },
    })
    rerender({ key: ['thread', 'b'] })

    emit('gateway://hold_changed')

    expect(invalidate).toHaveBeenCalledExactlyOnceWith({
      queryKey: ['thread', 'b'],
    })
  })

  it('unsubscribes on unmount', () => {
    const { unmount } = renderHook(() => useInvalidateOn(NAMES, KEY), {
      wrapper,
    })
    unmount()

    expect(unsubscribe).toHaveBeenCalledOnce()
  })
})
