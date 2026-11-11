import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useDevicePoll } from './useDevicePoll'

/** A poll the test settles by hand */
function deferredPoll() {
  const resolvers: ((answer: string) => void)[] = []
  const poll = vi.fn(
    () => new Promise<string>(resolve => resolvers.push(resolve))
  )
  return { poll, settle: (answer: string) => resolvers.shift()?.(answer) }
}

describe('useDevicePoll', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  function setup(session: string | null = 'flow-1') {
    const answers = deferredPoll()
    const onPolled = vi.fn<(answer: string) => number | null>(() => 1000)
    const hook = renderHook(
      ({ session }) =>
        useDevicePoll({
          session,
          delayMs: 1000,
          poll: answers.poll,
          onPolled,
        }),
      { initialProps: { session } }
    )
    return { ...answers, onPolled, ...hook }
  }

  it('does not poll without a session', async () => {
    const { poll } = setup(null)
    await vi.advanceTimersByTimeAsync(5000)
    expect(poll).not.toHaveBeenCalled()
  })

  it('polls after the delay and again after the wait onPolled returns', async () => {
    const { poll, settle, onPolled } = setup()

    await vi.advanceTimersByTimeAsync(999)
    expect(poll).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(poll).toHaveBeenCalledTimes(1)

    await act(async () => settle('waiting'))
    expect(onPolled).toHaveBeenCalledWith('waiting')
    await vi.advanceTimersByTimeAsync(1000)
    expect(poll).toHaveBeenCalledTimes(2)
  })

  it('does not ask again while the last poll is still running', async () => {
    const { poll } = setup()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(poll).toHaveBeenCalledTimes(1)
  })

  it('uses the wait the provider asked for', async () => {
    const { poll, settle, onPolled } = setup()
    onPolled.mockReturnValue(5000)
    await vi.advanceTimersByTimeAsync(1000)
    await act(async () => settle('slow down'))

    await vi.advanceTimersByTimeAsync(4999)
    expect(poll).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(poll).toHaveBeenCalledTimes(2)
  })

  it('stops when onPolled returns null', async () => {
    const { poll, settle, onPolled } = setup()
    onPolled.mockReturnValue(null)
    await vi.advanceTimersByTimeAsync(1000)
    await act(async () => settle('linked'))

    await vi.advanceTimersByTimeAsync(10_000)
    expect(poll).toHaveBeenCalledTimes(1)
  })

  it('drops the answer of a poll that was cancelled while in flight', async () => {
    const { poll, settle, onPolled, rerender } = setup()
    await vi.advanceTimersByTimeAsync(1000)
    expect(poll).toHaveBeenCalledTimes(1)

    rerender({ session: null })
    await act(async () => settle('linked'))

    expect(onPolled).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(poll).toHaveBeenCalledTimes(1)
  })

  it('starts over when the session changes', async () => {
    const { poll, rerender } = setup('flow-1')
    await vi.advanceTimersByTimeAsync(600)
    rerender({ session: 'flow-2' })

    await vi.advanceTimersByTimeAsync(600)
    expect(poll).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(400)
    expect(poll).toHaveBeenCalledTimes(1)
  })

  it('stops on unmount', async () => {
    const { poll, unmount } = setup()
    unmount()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(poll).not.toHaveBeenCalled()
  })
})
