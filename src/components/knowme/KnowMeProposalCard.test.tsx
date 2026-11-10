import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { KnowMeStatus } from '@/lib/bindings'
import {
  ApprovalRequestCard,
  type ApprovalCardData,
} from '@/components/agent-console/ApprovalRequestCard'
import { parseKnowMeProposal, withResponsibility } from './knowme-model'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const NOW = Date.now()

const STATUS: KnowMeStatus = {
  todoOpen: true,
  recheck: false,
  shownAt: NOW,
  answeredAt: null,
  threadId: 'inbox:knowMe:you',
  groups: [{ id: 'g_family', name: 'Family' }],
  recent: [],
}

const ARGS = {
  summary: 'Scrum master for two teams; a parent',
  responsibilities: [
    { text: 'Sprint planning', kind: 'work' },
    { text: 'School runs', kind: 'personal', onCard: true },
  ],
  canHelpWith: 'Sprint ceremonies and Jira questions.',
  notes: [
    { text: 'Two-week sprints' },
    { text: 'Children aged 4 and 7', private: true },
  ],
  suggest: [{ name: 'Sprint review prep', why: 'Prepares each review' }],
}

function card(args: ApprovalCardData['args'] = ARGS): ApprovalCardData {
  return {
    id: 'ap1',
    tool: 'knowme_propose',
    args,
    status: 'pending',
    createdAt: NOW,
    expiresAt: null,
  }
}

beforeEach(() => {
  Object.assign(commands, {
    getKnowMe: ok(STATUS),
    confirmKnowMeProposal: ok(null),
  })
})

describe('knowme-model', () => {
  it('reads a proposal: work on the card by default, personal never', () => {
    const proposal = parseKnowMeProposal('knowme_propose', ARGS)
    expect(proposal?.responsibilities).toEqual([
      {
        text: 'Sprint planning',
        kind: 'work',
        onCard: true,
        shareWithGroup: null,
      },
      {
        text: 'School runs',
        kind: 'personal',
        onCard: false,
        shareWithGroup: null,
      },
    ])
    expect(parseKnowMeProposal('skill_propose', ARGS)).toBeNull()
    if (!proposal) throw new Error('no proposal')
    // Switching an item to work drops its group; to personal, the card
    const shared = withResponsibility(proposal, 1, {
      shareWithGroup: 'g_family',
    })
    expect(shared.responsibilities[1]?.shareWithGroup).toBe('g_family')
    const work = withResponsibility(shared, 1, { kind: 'work' })
    expect(work.responsibilities[1]?.shareWithGroup).toBeNull()
    const personal = withResponsibility(proposal, 0, { kind: 'personal' })
    expect(personal.responsibilities[0]?.onCard).toBe(false)
  })
})

describe('a know-me proposal card', () => {
  it('shows each change, and confirms the person’s edits', async () => {
    const user = userEvent.setup()
    const onDecide = vi.fn().mockResolvedValue(true)
    render(<ApprovalRequestCard message={card()} onDecide={onDecide} />)
    const proposal = await screen.findByTestId('know-me-proposal')
    expect(proposal).toHaveTextContent('Scrum master for two teams; a parent')
    expect(proposal).toHaveTextContent('Sprint ceremonies and Jira questions.')
    expect(proposal).toHaveTextContent('Sprint review prep')
    expect(proposal).toHaveTextContent('Suggestions only')

    const [work, personal] = screen.getAllByTestId('know-me-responsibility')
    if (!work || !personal) throw new Error('two responsibilities expected')
    // Only work has the card switch
    expect(within(personal).queryByRole('switch')).toBeNull()
    await user.click(within(work).getByRole('switch'))
    expect(work).toHaveTextContent('Not on my card')
    // A personal item: private, or one of my groups
    await waitFor(() =>
      expect(
        within(personal).getByRole('combobox', { name: 'Who sees this' })
      ).toBeInTheDocument()
    )
    await user.selectOptions(
      within(personal).getByRole('combobox', { name: 'Who sees this' }),
      'g_family'
    )
    await user.click(
      screen.getByRole('button', { name: 'Remove Two-week sprints' })
    )

    await user.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() =>
      expect(mocks().confirmKnowMeProposal).toHaveBeenCalledTimes(1)
    )
    const confirm = commands.confirmKnowMeProposal as unknown as ReturnType<
      typeof vi.fn
    >
    const [approvalId, sent] = confirm.mock.calls[0] ?? [null, {}]
    expect(approvalId).toBe('ap1')
    expect(sent.responsibilities).toEqual([
      {
        text: 'Sprint planning',
        kind: 'work',
        onCard: false,
        shareWithGroup: null,
      },
      {
        text: 'School runs',
        kind: 'personal',
        onCard: false,
        shareWithGroup: 'g_family',
      },
    ])
    expect(sent.notes).toEqual([
      { text: 'Children aged 4 and 7', private: true },
    ])
    expect(onDecide).not.toHaveBeenCalled()
  })

  it('refuses without writing anything', async () => {
    const user = userEvent.setup()
    const onDecide = vi.fn().mockResolvedValue(true)
    render(<ApprovalRequestCard message={card()} onDecide={onDecide} />)
    await user.click(await screen.findByRole('button', { name: 'Refuse' }))
    expect(onDecide).toHaveBeenCalledWith('ap1', 'rejected')
    expect(mocks().confirmKnowMeProposal).not.toHaveBeenCalled()
  })

  it('is read-only once decided', async () => {
    render(<ApprovalRequestCard message={{ ...card(), status: 'approved' }} />)
    const proposal = await screen.findByTestId('know-me-proposal')
    expect(
      within(proposal).queryByRole('button', { name: 'Confirm' })
    ).toBeNull()
    expect(within(proposal).queryByRole('textbox')).toBeNull()
  })
})
