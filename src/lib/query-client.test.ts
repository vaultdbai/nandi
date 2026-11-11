import { describe, expect, it } from 'vitest'
import { queryClient } from './query-client'

describe('queryClient', () => {
  it('never retries a failed mutation', () => {
    expect(queryClient.getDefaultOptions().mutations?.retry).toBe(false)
  })
})
