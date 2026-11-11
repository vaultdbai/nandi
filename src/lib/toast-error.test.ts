import { beforeEach, describe, expect, it, vi } from 'vitest'
import { toast } from 'sonner'
import { RELAY_UNAVAILABLE } from '@/lib/bindings'
import { toastRelayActionError } from './toast-error'

vi.mock('sonner', () => {
  const toast = Object.assign(vi.fn(), { error: vi.fn() })
  return { toast }
})

describe('toastRelayActionError', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it.each([
    ['a string', `${RELAY_UNAVAILABLE}: no relay`],
    ['an Error', new Error(`${RELAY_UNAVAILABLE}: no relay`)],
  ])('shows the calm connect notice, not an error, for %s', (_, error) => {
    toastRelayActionError(error)

    expect(toast).toHaveBeenCalledOnce()
    expect(toast.error).not.toHaveBeenCalled()
  })

  it.each([
    [
      'a string',
      'the hold was already decided',
      'the hold was already decided',
    ],
    ['an Error', new Error('boom'), 'boom'],
  ])('shows the error text for %s', (_, error, text) => {
    toastRelayActionError(error)

    expect(toast.error).toHaveBeenCalledWith(text)
    expect(toast).not.toHaveBeenCalled()
  })
})
