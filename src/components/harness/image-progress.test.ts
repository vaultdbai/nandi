import { describe, expect, it } from 'vitest'
import { formatDownload, progressPercent } from './image-progress'

describe('image progress', () => {
  it('reads a download size in the user’s units', () => {
    expect(formatDownload(450 * 1024 * 1024, 'en')).toBe('450 MB')
    expect(formatDownload(null, 'en')).toBeNull()
    expect(formatDownload(0, 'en')).toBeNull()
  })

  it('turns bytes into a percentage once the total is known', () => {
    expect(progressPercent({ doneBytes: 25, totalBytes: 100 })).toBe(25)
    expect(progressPercent({ doneBytes: 120, totalBytes: 100 })).toBe(100)
    expect(progressPercent({ doneBytes: 25, totalBytes: null })).toBeNull()
    expect(progressPercent(null)).toBeNull()
  })
})
