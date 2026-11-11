import { describe, expect, it, vi } from 'vitest'
import type * as TauriBindings from './tauri-bindings'

// The global setup swaps in mocks for the bindings; this file tests the real helper
const { unwrapResult } =
  await vi.importActual<typeof TauriBindings>('./tauri-bindings')

describe('unwrapResult', () => {
  it('returns the data of an ok result', () => {
    expect(unwrapResult({ status: 'ok', data: 7 })).toBe(7)
  })

  it('throws an Error whose message is the core error text', () => {
    expect(() => unwrapResult({ status: 'error', error: 'no relay' })).toThrow(
      new Error('no relay')
    )
  })

  it('throws a non-string error as it is', () => {
    const typed = { kind: 'refused' }
    let thrown: unknown
    try {
      unwrapResult({ status: 'error', error: typed })
    } catch (error) {
      thrown = error
    }
    expect(thrown).toBe(typed)
  })
})
