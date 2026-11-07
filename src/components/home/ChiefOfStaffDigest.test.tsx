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
    ...overrides,
  }
}

describe('ChiefOfStaffDigest', () => {
  let originalGet: unknown
  let originalSeen: unknown
  let originalApprove: unknown
  let originalDismiss: unknown
  beforeEach(() => {
    originalGet = mocked.getChiefOfStaffDigest
    originalSeen = mocked.markChiefOfStaffDigestSeen
    originalApprove = mocked.approveQueuedWork
    originalDismiss = mocked.dismissQueuedWork
  })
  afterEach(() => {
    mocked.getChiefOfStaffDigest = originalGet
    mocked.markChiefOfStaffDigestSeen = originalSeen
    mocked.approveQueuedWork = originalApprove
    mocked.dismissQueuedWork = originalDismiss
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
})
