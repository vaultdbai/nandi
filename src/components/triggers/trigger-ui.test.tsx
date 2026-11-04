import { describe, expect, it } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { FireStatusBadge } from './trigger-ui'

describe('FireStatusBadge', () => {
  it("shows the run's status once the fire dispatched one", () => {
    render(
      <FireStatusBadge fire={{ status: 'queued', runStatus: 'completed' }} />
    )
    expect(screen.getByText('Completed')).toBeInTheDocument()
    expect(screen.queryByText('Queued')).toBeNull()
  })

  it("shows the fire's own status without a run, or when it didn't run", () => {
    const { rerender } = render(
      <FireStatusBadge fire={{ status: 'queued', runStatus: null }} />
    )
    expect(screen.getByText('Queued')).toBeInTheDocument()
    rerender(
      <FireStatusBadge fire={{ status: 'rateLimited', runStatus: null }} />
    )
    expect(screen.getByText('Rate limited')).toBeInTheDocument()
  })
})
