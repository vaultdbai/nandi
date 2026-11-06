import { describe, expect, it, vi } from 'vitest'
import { waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { commands } from '@/lib/tauri-bindings'
import { render, screen } from '@/test/test-utils'
import {
  ApprovalRequestCard,
  type ApprovalCardData,
} from './ApprovalRequestCard'

const NOW = Date.now()

function handOver(
  status: ApprovalCardData['status'] = 'pending'
): ApprovalCardData {
  return {
    id: 'ap1',
    tool: 'pilot_browser__browser_hand_over',
    args: {
      site: 'LinkedIn',
      summary: 'Launch post for Acme',
      draft: 'We launched today.\n#acme',
    },
    status,
    createdAt: NOW,
    expiresAt: NOW + 24 * 3_600_000,
  }
}

describe('HandOverCard', () => {
  it('shows the draft and where to finish it, not Approve and Reject', () => {
    render(
      <ApprovalRequestCard
        message={handOver()}
        agentName="Social Manager"
        onDecide={vi.fn()}
      />
    )
    expect(screen.getByTestId('hand-over-card')).toBeInTheDocument()
    expect(
      screen.getByText('Social Manager · Your turn: review and post')
    ).toBeInTheDocument()
    expect(screen.getByText(/Launch post for Acme/)).toBeInTheDocument()
    expect(screen.getByText(/on LinkedIn/)).toBeInTheDocument()
    expect(screen.getByText(/We launched today\./)).toBeInTheDocument()
    expect(screen.getByText('Waits up to 24 h more')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Approve/ })).toBeNull()
  })

  it('answers Done and Skip', async () => {
    const user = userEvent.setup()
    render(<ApprovalRequestCard message={handOver()} onDecide={vi.fn()} />)
    await user.click(screen.getByRole('button', { name: /Done, I posted it/ }))
    await waitFor(() =>
      expect(commands.answerHandOver).toHaveBeenCalledWith('ap1', {
        outcome: 'done',
        finalText: null,
      })
    )
    // Waiting for Rust's approval_resolved: no double answer
    expect(screen.getByRole('button', { name: /Skip/ })).toBeDisabled()
  })

  it('answers Changed it with the text that went out', async () => {
    const user = userEvent.setup()
    render(<ApprovalRequestCard message={handOver()} onDecide={vi.fn()} />)
    await user.click(screen.getByRole('button', { name: /Changed it/ }))
    const text = screen.getByLabelText('What went out')
    expect(text).toHaveValue('We launched today.\n#acme')
    await user.clear(text)
    await user.type(text, 'We shipped it!')
    await user.click(screen.getByRole('button', { name: /Save/ }))
    await waitFor(() =>
      expect(commands.answerHandOver).toHaveBeenCalledWith('ap1', {
        outcome: 'changed',
        finalText: 'We shipped it!',
      })
    )
  })

  it('is read-only in a transcript and once answered', () => {
    const { unmount } = render(<ApprovalRequestCard message={handOver()} />)
    expect(screen.queryByRole('button', { name: /Done/ })).toBeNull()
    unmount()
    render(
      <ApprovalRequestCard message={handOver('rejected')} onDecide={vi.fn()} />
    )
    expect(screen.getByText('Skipped')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Skip$/ })).toBeNull()
  })
})
