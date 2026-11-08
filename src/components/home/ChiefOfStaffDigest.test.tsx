import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test/test-utils'
import userEvent from '@testing-library/user-event'
import { commands } from '@/lib/tauri-bindings'
import type { ChiefOfStaffDigest, DigestItem, QueuedWork } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { ChiefOfStaffDigest as DigestCard } from './ChiefOfStaffDigest'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

function item(overrides: Partial<DigestItem> = {}): DigestItem {
  return {
    requestId: 'r1',
    title: 'Pay the power bill',
    source: 'email',
    kind: 'started',
    status: 'running',
    text: null,
    at: Date.now() - 5 * 60 * 1000,
    threadId: null,
    waitingStep: null,
    workflowProposal: null,
    ...overrides,
  }
}

const digest = (
  items: DigestItem[],
  since: number | null = 1,
  queue: QueuedWork[] = []
): ChiefOfStaffDigest => ({ since, items, queue })

function queued(overrides: Partial<QueuedWork> = {}): QueuedWork {
  return {
    id: 'q1',
    at: Date.now() - 5 * 60 * 1000,
    source: 'email',
    action: 'start',
    requestId: null,
    requestTitle: null,
    text: 'Your power bill is due Friday',
    sender: 'bills@power.example',
    screening: { verdict: 'clean', reasons: [], links: [] },
    status: 'pending',
    resultRequestId: null,
    threadId: 'intake:q1',
    plan: null,
    ...overrides,
  }
}

describe('ChiefOfStaffDigest', () => {
  let originalGet: unknown
  let originalSeen: unknown
  let originalApprove: unknown
  let originalDismiss: unknown
  let originalComplete: unknown
  beforeEach(() => {
    originalGet = mocked.getChiefOfStaffDigest
    originalSeen = mocked.markChiefOfStaffDigestSeen
    originalApprove = mocked.approveQueuedWork
    originalDismiss = mocked.dismissQueuedWork
    originalComplete = mocked.completeRequestStep
  })
  afterEach(() => {
    mocked.getChiefOfStaffDigest = originalGet
    mocked.markChiefOfStaffDigestSeen = originalSeen
    mocked.approveQueuedWork = originalApprove
    mocked.dismissQueuedWork = originalDismiss
    mocked.completeRequestStep = originalComplete
  })

  it('renders nothing without items', async () => {
    const get = vi.fn(async () => ok(digest([])))
    mocked.getChiefOfStaffDigest = get
    render(<DigestCard />)
    await vi.waitFor(() => expect(get).toHaveBeenCalled())
    expect(screen.queryByTestId('home-cos-digest')).toBeNull()
  })

  it('shows the summary and rows, and opens the Request', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest([
          item({ kind: 'needsYou', status: 'waiting' }),
          item({
            requestId: 'r2',
            title: 'Dentist',
            kind: 'updated',
            text: 'Moved to Monday',
          }),
        ])
      )
    )
    render(<DigestCard />)
    expect(await screen.findByText('Since you were away')).toBeInTheDocument()
    expect(screen.getByText('1 needs you · 1 updated')).toBeInTheDocument()
    expect(screen.getByText('Moved to Monday')).toBeInTheDocument()
    await userEvent.click(screen.getByText('Dentist'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'request',
      requestId: 'r2',
    })
  })

  it('offers to save recurring finished work as a workflow, with the draft to check', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest([
          item({
            kind: 'finished',
            status: 'done',
            title: 'Pay the power bill for November',
            workflowProposal: {
              requestId: 'r1',
              reason: 'sameWork',
              likeRequestId: 'r0',
              likeTitle: 'Pay the power bill for October',
            },
          }),
        ])
      )
    )
    const draft = vi.fn(async () => ok('id: pay\nname: Pay\nsteps: []\n'))
    const originalDraft = mocked.draftWorkflowFromRequest
    mocked.draftWorkflowFromRequest = draft
    render(<DigestCard />)
    expect(
      await screen.findByText(
        'Done before as “Pay the power bill for October”. Save it as a workflow?'
      )
    ).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('home-cos-save-workflow'))
    await vi.waitFor(() => expect(draft).toHaveBeenCalledWith('r1'))
    expect(await screen.findByText('Save as a workflow')).toBeInTheDocument()
    mocked.draftWorkflowFromRequest = originalDraft
  })

  it('uses the plain title when nothing was seen yet, and Got it marks seen', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () => ok(digest([item()], null)))
    const seen = vi.fn(async () => ok(null))
    mocked.markChiefOfStaffDigestSeen = seen
    render(<DigestCard />)
    expect(
      await screen.findByText('What your Chief of Staff did')
    ).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Got it' }))
    expect(seen).toHaveBeenCalled()
  })

  it('shows a clean queued item and approves it with one click', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(digest([], 1, [queued()]))
    )
    const approve = vi.fn(async () => ok('r7'))
    mocked.approveQueuedWork = approve
    render(<DigestCard />)
    expect(await screen.findByText('Waiting for your OK')).toBeInTheDocument()
    expect(screen.getByText('1 waiting for your OK')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Got it' })).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: 'Approve' }))
    await vi.waitFor(() => expect(approve).toHaveBeenCalledWith('q1'))
  })

  it('labels news as Add to the Request', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest([], 1, [
          queued({
            action: 'update',
            requestId: 'r1',
            requestTitle: 'Dentist',
          }),
        ])
      )
    )
    render(<DigestCard />)
    expect(await screen.findByText('Add to: Dentist')).toBeInTheDocument()
  })

  it('cannot approve a blocked item, only dismiss it', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest([], 1, [
          queued({
            screening: {
              verdict: 'blocked',
              reasons: ['Link to a private address'],
              links: ['http://10.0.0.7/login'],
            },
          }),
        ])
      )
    )
    const dismiss = vi.fn(async () => ok(null))
    mocked.dismissQueuedWork = dismiss
    render(<DigestCard />)
    expect(await screen.findByText('Blocked by screening')).toBeInTheDocument()
    expect(screen.getByText('Link to a private address')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Approve' })).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    await vi.waitFor(() => expect(dismiss).toHaveBeenCalledWith('q1'))
  })

  it('asks before approving a suspicious item', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest([], 1, [
          queued({
            screening: {
              verdict: 'suspicious',
              reasons: ['Asks for a password'],
              links: [],
            },
          }),
        ])
      )
    )
    const approve = vi.fn(async () => ok('r7'))
    mocked.approveQueuedWork = approve
    render(<DigestCard />)
    await userEvent.click(
      await screen.findByRole('button', { name: 'Approve' })
    )
    expect(approve).not.toHaveBeenCalled()
    await userEvent.click(
      await screen.findByRole('button', { name: 'Approve anyway' })
    )
    await vi.waitFor(() => expect(approve).toHaveBeenCalledWith('q1'))
  })

  it('opens the thread of a row and of a queued item', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest(
          [item({ title: 'Dentist', requestId: 'r2', threadId: 'intake:d' })],
          1,
          [queued()]
        )
      )
    )
    render(<DigestCard />)
    await userEvent.click(await screen.findByText('Dentist'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'intake-thread',
      conversationId: 'intake:d',
    })
    await userEvent.click(screen.getByText('Your power bill is due Friday'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'intake-thread',
      conversationId: 'intake:q1',
    })
  })

  it('marks the waiting step done from a needs-you row', async () => {
    mocked.getChiefOfStaffDigest = vi.fn(async () =>
      ok(
        digest([
          item({
            kind: 'needsYou',
            status: 'waiting',
            waitingStep: { position: 2, label: 'Call the bank' },
          }),
          item({ requestId: 'r9', title: 'No step' }),
        ])
      )
    )
    const complete = vi.fn(async () => ok(null))
    mocked.completeRequestStep = complete
    render(<DigestCard />)
    expect(
      await screen.findAllByRole('button', { name: /Mark done/ })
    ).toHaveLength(1)
    await userEvent.click(
      screen.getByRole('button', { name: 'Mark done: Call the bank' })
    )
    await vi.waitFor(() => expect(complete).toHaveBeenCalledWith('r1', 2))
  })
})
