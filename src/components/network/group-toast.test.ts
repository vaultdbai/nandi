import { describe, expect, it, vi } from 'vitest'
import { toast } from 'sonner'
import { toastGroupError } from './group-toast'

vi.mock('sonner', () => {
  const toast = Object.assign(vi.fn(), { error: vi.fn() })
  return { toast }
})

describe('toastGroupError', () => {
  it('tells a daily limit calmly, with when it resets', () => {
    const resetsAt = Date.UTC(2026, 10, 11)
    toastGroupError(
      "Couldn't create the group",
      `daily_limit@${resetsAt}: You've sent the 500 messages a day`
    )
    expect(toast.error).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledWith("Couldn't create the group", {
      description: expect.stringContaining(
        "You've sent today's messages on the public relay. You can send again from"
      ),
    })
  })

  it("keeps the relay's own words for other refusals", () => {
    toastGroupError(
      "Couldn't create the group",
      "That's past this relay's group limits, which count per person (all your Nandis together): 10 groups"
    )
    expect(toast.error).toHaveBeenCalledWith("Couldn't create the group", {
      description: expect.stringContaining('count per person'),
    })
  })
})
