import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import {
  ApprovalRequestCard,
  type ApprovalCardData,
} from './ApprovalRequestCard'

const NOW = Date.now()

const ARGS = {
  name: 'pay-bills',
  description: 'Pay the household bills',
  when: 'A bill is due',
  body: '1. Open the site.\n2. Pay from Chase, never the card.',
  reason: 'You told me how you pay bills',
}

function proposal(args: ApprovalCardData['args'] = ARGS): ApprovalCardData {
  return {
    id: 'ap1',
    tool: 'skill_propose',
    args,
    status: 'pending',
    createdAt: NOW,
    expiresAt: NOW + 24 * 3_600_000,
  }
}

describe('PlaybookProposalPreview on the approval card', () => {
  it('shows a new playbook with its steps folded, and Approve / Reject', async () => {
    const user = userEvent.setup()
    render(
      <ApprovalRequestCard
        message={proposal()}
        agentName="Chief of Staff"
        onDecide={vi.fn()}
      />
    )
    expect(
      screen.getByText('Save a new playbook: pay-bills')
    ).toBeInTheDocument()
    expect(
      screen.getByText('Why: You told me how you pay bills')
    ).toBeInTheDocument()
    expect(screen.getByText('Pay the household bills')).toBeInTheDocument()
    expect(screen.getByText('A bill is due')).toBeInTheDocument()
    expect(screen.queryByTestId('playbook-proposal-body')).toBeNull()
    await user.click(screen.getByText('Show the steps'))
    expect(screen.getByTestId('playbook-proposal-body')).toHaveTextContent(
      'Pay from Chase, never the card.'
    )
    expect(screen.getByRole('button', { name: /Approve/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Reject/ })).toBeInTheDocument()
  })

  it('shows a change against the playbook it replaces', async () => {
    const user = userEvent.setup()
    render(
      <ApprovalRequestCard
        message={proposal({
          ...ARGS,
          current: {
            description: 'Pay bills',
            when: 'A bill is due',
            body: '1. Open the site.\n2. Pay by card.',
          },
        })}
        onDecide={vi.fn()}
      />
    )
    expect(
      screen.getByText(
        'Change the playbook pay-bills (replaces the current one)'
      )
    ).toBeInTheDocument()
    expect(screen.getByText('Was: Pay bills')).toBeInTheDocument()
    await user.click(screen.getByText('Show the changes'))
    const body = screen.getByTestId('playbook-proposal-body')
    const kinds = Array.from(body.querySelectorAll('[data-kind]')).map(
      line => `${line.getAttribute('data-kind')}:${line.textContent}`
    )
    expect(kinds).toEqual([
      'same:  1. Open the site.',
      'removed:- 2. Pay by card.',
      'added:+ 2. Pay from Chase, never the card.',
    ])
  })
})
