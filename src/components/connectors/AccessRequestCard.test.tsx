import { afterEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ApprovalRequestCard } from '@/components/agent-console/ApprovalRequestCard'
import type { JsonValue } from '@/lib/bindings'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const original = mocked.answerAccessRequest

const ARGS = {
  agent: 'chief-of-staff',
  family: 'email',
  connectors: [{ id: 'outlook', name: 'Outlook' }],
  access: 'read',
  tool: 'email_search',
  details: ['Mail in your inbox'],
  askEveryTime: false,
}

function card(
  args: Record<string, unknown> = ARGS,
  status: 'pending' | 'approved' = 'pending',
  interactive = true
) {
  return (
    <ApprovalRequestCard
      message={{
        id: 'call-1#access',
        tool: 'connector_access',
        args: args as JsonValue,
        status,
        createdAt: Date.now(),
        expiresAt: null,
      }}
      agentName="Chief of Staff"
      onDecide={interactive ? () => Promise.resolve(true) : undefined}
    />
  )
}

const SITE_ARGS = {
  agent: 'browser',
  family: 'site',
  connectors: [],
  access: 'read',
  tool: 'pilot_browser__browser_navigate',
  details: [],
  askEveryTime: false,
  site: {
    host: 'xn--bcher-kva.example',
    display: 'bücher.example',
    pilot: 'browser',
    reason: 'Find the opening hours',
    canSave: true,
    subdomains: false,
  },
}

describe('AccessRequestCard for a site', () => {
  afterEach(() => {
    mocked.answerAccessRequest = original
  })

  it('names the agent, the site decoded and raw, and why', () => {
    render(card(SITE_ARGS))
    expect(
      screen.getByText('Chief of Staff wants to open bücher.example')
    ).toBeInTheDocument()
    expect(screen.getByTestId('access-site-raw')).toHaveTextContent(
      'Address: xn--bcher-kva.example'
    )
    expect(screen.getByText('Why: Find the opening hours')).toBeInTheDocument()
    expect(screen.queryByLabelText('Saved (backed up)')).not.toBeInTheDocument()
  })

  it('allows once, the host only by default', async () => {
    const answer = vi.fn().mockResolvedValue({ status: 'ok', data: null })
    mocked.answerAccessRequest = answer
    render(card(SITE_ARGS))
    expect(
      screen.getByLabelText('Include its subdomains (like www.bücher.example)')
    ).not.toBeChecked()
    await userEvent.click(screen.getByRole('button', { name: 'Allow once' }))
    await waitFor(() =>
      expect(answer).toHaveBeenCalledWith('call-1#access', 'allowOnce', false)
    )
  })

  it('always allows for the agent, with its subdomains when ticked', async () => {
    const answer = vi.fn().mockResolvedValue({ status: 'ok', data: null })
    mocked.answerAccessRequest = answer
    render(card(SITE_ARGS))
    await userEvent.click(
      screen.getByLabelText('Include its subdomains (like www.bücher.example)')
    )
    await userEvent.click(
      screen.getByRole('button', { name: 'Always allow for Chief of Staff' })
    )
    await waitFor(() =>
      expect(answer).toHaveBeenCalledWith('call-1#access', 'allowSaved', true)
    )
    expect(
      await screen.findByText('Always allowed for this agent')
    ).toBeInTheDocument()
  })

  it("offers only once for an agent whose file can't change", () => {
    render(card({ ...SITE_ARGS, site: { ...SITE_ARGS.site, canSave: false } }))
    expect(
      screen.queryByRole('button', { name: /Always allow/ })
    ).not.toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Allow once' })
    ).toBeInTheDocument()
  })
})

describe('AccessRequestCard', () => {
  afterEach(() => {
    mocked.answerAccessRequest = original
  })

  it('says who wants what, with the details', () => {
    render(card())
    expect(
      screen.getByText('Chief of Staff wants to read your email (Outlook)')
    ).toBeInTheDocument()
    expect(screen.getByText('Mail in your inbox')).toBeInTheDocument()
  })

  it('allows for this agent as saved by default', async () => {
    const answer = vi.fn().mockResolvedValue({ status: 'ok', data: null })
    mocked.answerAccessRequest = answer
    render(card())
    expect(screen.getByLabelText('Saved (backed up)')).toBeChecked()
    await userEvent.click(
      screen.getByRole('button', { name: 'Allow for this agent' })
    )
    await waitFor(() =>
      expect(answer).toHaveBeenCalledWith('call-1#access', 'allowSaved', false)
    )
    expect(
      await screen.findByText('Allowed for this agent (saved)')
    ).toBeInTheDocument()
  })

  it('can keep the grant on this computer only', async () => {
    const answer = vi.fn().mockResolvedValue({ status: 'ok', data: null })
    mocked.answerAccessRequest = answer
    render(card())
    await userEvent.click(screen.getByLabelText('This computer only'))
    await userEvent.click(
      screen.getByRole('button', { name: 'Allow for this agent' })
    )
    await waitFor(() =>
      expect(answer).toHaveBeenCalledWith('call-1#access', 'allowLocal', false)
    )
  })

  it('offers only once when a policy asks every time', async () => {
    const answer = vi.fn().mockResolvedValue({ status: 'ok', data: null })
    mocked.answerAccessRequest = answer
    render(card({ ...ARGS, askEveryTime: true }))
    expect(
      screen.queryByRole('button', { name: 'Allow for this agent' })
    ).not.toBeInTheDocument()
    expect(screen.getByText(/A policy asks every time/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: "Don't allow" }))
    await waitFor(() =>
      expect(answer).toHaveBeenCalledWith('call-1#access', 'deny', false)
    )
  })

  it('shows the decision and who made it, read-only', () => {
    render(
      card(
        { ...ARGS, choice: 'allowSaved', decidedBy: 'Dev' },
        'approved',
        false
      )
    )
    expect(
      screen.getByText('Allowed for this agent (saved) by Dev')
    ).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: 'Allow once' })
    ).not.toBeInTheDocument()
  })

  it('has no answers in a read-only transcript', () => {
    render(card(ARGS, 'pending', false))
    expect(
      screen.queryByRole('button', { name: 'Allow once' })
    ).not.toBeInTheDocument()
  })
})
