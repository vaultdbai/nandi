import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { Request } from '@/lib/bindings'
import {
  blocker,
  request as fixture,
  step,
  system,
} from '@/components/requests/request-fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { RequestDetailView } from './RequestDetailView'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const fail = (error: string) =>
  vi.fn().mockResolvedValue({ status: 'error', error })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

function show(request: Request, stepId?: string) {
  Object.assign(commands, { getRequest: ok(request) })
  return render(<RequestDetailView requestId={request.id} stepId={stepId} />)
}

describe('RequestDetailView', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    Object.assign(commands, {
      cancelTask: ok(null),
      retryTask: ok(fixture()),
      setTrustRelationship: ok(null),
      pauseDelegationsFromMember: ok(null),
      approveAction: ok(null),
      rateWork: ok(null),
      getAgentDelegation: ok({
        allowOutbound: false,
        acceptInbound: false,
        maxConcurrent: null,
        overrides: [],
      }),
      setAgentDelegation: ok(null),
    })
  })

  it('shows the header, system steps and plan steps of an incoming request', async () => {
    show(fixture())
    expect(
      await screen.findByRole('heading', {
        name: 'Summarize Q3 portfolio risk',
      })
    ).toBeInTheDocument()
    expect(
      screen.getByText("From Priya's Nandi · Incoming")
    ).toBeInTheDocument()
    expect(screen.getByText('step 1 of 1')).toBeInTheDocument()
    const rail = screen.getByRole('list', { name: 'Steps' })
    expect(within(rail).getByText('Received')).toBeInTheDocument()
    expect(within(rail).getByText('Routed')).toBeInTheDocument()
    expect(within(rail).getByText('Portfolio Analyst')).toBeInTheDocument()
    expect(
      within(rail).getByText("Delivered to Priya's Nandi")
    ).toBeInTheDocument()
    // No banner when nothing is stuck
    expect(screen.queryByTestId('request-banner')).toBeNull()
    // Never a raw ID in the header
    expect(screen.getByRole('banner')).not.toHaveTextContent('t1')
  })

  it('shows one banner and approves without asking first', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        status: 'waiting',
        stuck: blocker('needsMyApproval', { approvalId: 'ap1', runId: 'r1' }),
      })
    )
    const banner = await screen.findByTestId('request-banner')
    expect(banner).toHaveTextContent(
      'Portfolio Analyst needs your approval to go on'
    )
    expect(screen.getAllByTestId('request-banner')).toHaveLength(1)
    await user.click(within(banner).getByRole('button', { name: 'Approve' }))
    await waitFor(() =>
      expect(mocks().approveAction).toHaveBeenCalledWith(
        'papers',
        'ap1',
        'approved'
      )
    )
  })

  it('asks before rejecting', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        status: 'waiting',
        stuck: blocker('needsMyApproval', { approvalId: 'ap1' }),
      })
    )
    await user.click(await screen.findByRole('button', { name: 'Reject' }))
    expect(screen.getByText('Reject this step?')).toBeInTheDocument()
    expect(mocks().approveAction).not.toHaveBeenCalled()
    await user.click(
      within(screen.getByRole('alertdialog')).getByRole('button', {
        name: 'Reject',
      })
    )
    await waitFor(() =>
      expect(mocks().approveAction).toHaveBeenCalledWith(
        'papers',
        'ap1',
        'rejected'
      )
    )
  })

  it('offers to trust an untrusted member by name, after asking', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        status: 'failed',
        steps: [
          system('received', 'received'),
          system('allowed', 'allowed', { status: 'failed' }),
        ],
        planSteps: 0,
        stuck: blocker('notTrusted', {
          terminal: true,
          stepId: 'allowed',
          reason: 'Provider requires trust. (raw text)',
        }),
      })
    )
    const banner = await screen.findByTestId('request-banner')
    expect(banner).toHaveTextContent(
      "Priya's Nandi isn't trusted to send you requests"
    )
    // The raw reason stays out of the banner
    expect(banner).not.toHaveTextContent('Provider requires trust')
    expect(banner).not.toHaveTextContent('priya-laptop')
    await user.click(
      within(banner).getByRole('button', { name: "Trust Priya's Nandi" })
    )
    await user.click(
      within(screen.getByRole('alertdialog')).getByRole('button', {
        name: "Trust Priya's Nandi",
      })
    )
    await waitFor(() =>
      expect(mocks().setTrustRelationship).toHaveBeenCalledWith(
        'priya-laptop',
        {
          addedAsTrustedPeer: true,
          delegationsPaused: null,
        }
      )
    )
  })

  it('on the requester side, asks the owner to trust you', async () => {
    show(
      fixture({
        direction: 'outgoing',
        status: 'failed',
        stuck: blocker('notTrusted', { who: 'them', terminal: true }),
      })
    )
    const banner = await screen.findByTestId('request-banner')
    expect(banner).toHaveTextContent("Priya's Nandi doesn't trust you yet")
    expect(
      within(banner).getByRole('button', {
        name: "Ask Priya's Nandi to trust you",
      })
    ).toBeInTheDocument()
  })

  it('opens Settings › Models for a missing model', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        status: 'failed',
        stuck: blocker('noModel', { terminal: true }),
      })
    )
    await user.click(await screen.findByRole('button', { name: 'Add a model' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'models',
    })
  })

  it('tries the next provider for a timed-out outgoing request', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        direction: 'outgoing',
        status: 'failed',
        stuck: blocker('timedOut', { terminal: true }),
      })
    )
    await user.click(
      await screen.findByRole('button', { name: 'Try next provider' })
    )
    await waitFor(() => expect(mocks().retryTask).toHaveBeenCalledWith('t1'))
  })

  it('stacks failover attempts under the peer step', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        direction: 'outgoing',
        steps: [
          system('sent', 'sent'),
          step({
            kind: 'peer',
            label: "Priya's Nandi",
            progress: 'Scoring positions',
            attempts: [
              {
                taskId: 't1',
                attemptNumber: 1,
                provider: "Sam's Nandi",
                state: 'timedOut',
                startedAt: 1_000,
                endedAt: 2_000,
                error: 'late',
              },
              {
                taskId: 't2',
                attemptNumber: 2,
                provider: "Priya's Nandi",
                state: 'working',
                startedAt: 2_000,
                endedAt: null,
                error: null,
              },
            ],
          }),
        ],
      })
    )
    await user.click(
      await screen.findByRole('button', { name: /Priya's Nandi/ })
    )
    const attempts = screen.getByRole('list', { name: 'Attempts' })
    expect(within(attempts).getByText("Sam's Nandi")).toBeInTheDocument()
    expect(within(attempts).getByText('Attempt 2')).toBeInTheDocument()
    expect(screen.getByText(/Scoring positions/)).toBeInTheDocument()
  })

  it('asks before stopping and cancels the task', async () => {
    const user = userEvent.setup()
    show(fixture())
    await user.click(await screen.findByRole('button', { name: 'Stop' }))
    expect(
      screen.getByText("Priya's Nandi is told to stop.")
    ).toBeInTheDocument()
    await user.click(
      within(screen.getByRole('alertdialog')).getByRole('button', {
        name: 'Stop',
      })
    )
    await waitFor(() =>
      expect(mocks().cancelTask).toHaveBeenCalledWith('t1-gw', null)
    )
  })

  it('puts the result first when done and rates an outgoing request', async () => {
    const user = userEvent.setup()
    show(
      fixture({
        direction: 'outgoing',
        status: 'done',
        endedAt: 3_000,
        result: 'Q3 risk is concentrated in two positions',
      })
    )
    expect(
      await screen.findByText('Q3 risk is concentrated in two positions')
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '4 stars' }))
    await waitFor(() => expect(mocks().rateWork).toHaveBeenCalledWith('t1', 4))
    expect(await screen.findByText('You rated it 4 ★')).toBeInTheDocument()
  })

  it('lists activity and keeps raw IDs in Details', async () => {
    const user = userEvent.setup()
    show(fixture())
    await user.click(await screen.findByRole('tab', { name: 'Activity' }))
    const activity = screen.getByRole('list', { name: 'Activity' })
    expect(within(activity).getByText('— → submitted')).toBeInTheDocument()
    await user.click(screen.getByRole('tab', { name: 'Details' }))
    expect(screen.getByText('t1-gw')).toBeInTheDocument()
    expect(screen.getByText('priya-laptop')).toBeInTheDocument()
  })

  it('says when a request is gone', async () => {
    Object.assign(commands, { getRequest: fail('Request x not found') })
    render(<RequestDetailView requestId="x" />)
    expect(
      await screen.findByText('This request no longer exists')
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Back to the dashboard' })
    ).toBeInTheDocument()
  })

  describe('a local ask', () => {
    const local = (overrides: Partial<Request> = {}) =>
      fixture({
        id: 'r1',
        title: 'Onboard Sam',
        direction: 'local',
        capability: '',
        counterpart: null,
        stuck: null,
        steps: [
          step({
            id: 'step-1',
            kind: 'person',
            label: 'Prepare a laptop',
            status: 'waiting',
          }),
          step({
            id: 'step-2',
            kind: 'agent',
            label: 'Write the welcome note',
            status: 'pending',
          }),
        ],
        planSteps: 2,
        currentStep: 1,
        status: 'waiting',
        endedAt: null,
        ...overrides,
      })

    it('marks a step waiting on you done', async () => {
      Object.assign(commands, { completeRequestStep: ok(null) })
      const user = userEvent.setup()
      show(local())
      expect(await screen.findByText('On this Nandi')).toBeInTheDocument()
      await user.click(screen.getByRole('button', { name: 'Mark done' }))
      await waitFor(() =>
        expect(mocks().completeRequestStep).toHaveBeenCalledWith('r1', 0)
      )
    })

    it('stops the Request itself', async () => {
      Object.assign(commands, { cancelRequest: ok(null) })
      const user = userEvent.setup()
      show(local())
      await user.click(await screen.findByRole('button', { name: 'Stop' }))
      const dialog = await screen.findByRole('alertdialog')
      await user.click(within(dialog).getByRole('button', { name: 'Stop' }))
      await waitFor(() =>
        expect(mocks().cancelRequest).toHaveBeenCalledWith('r1')
      )
      expect(mocks().cancelTask).not.toHaveBeenCalled()
    })

    it('does it again as a new Request', async () => {
      Object.assign(commands, { rerunRequest: ok('r2') })
      const user = userEvent.setup()
      show(local({ status: 'done', endedAt: Date.now() }))
      await user.click(
        await screen.findByRole('button', { name: 'Do this again' })
      )
      await waitFor(() =>
        expect(useNavigationStore.getState().current).toEqual({
          kind: 'request',
          requestId: 'r2',
        })
      )
    })

    it('saves it as a workflow after you confirm the draft', async () => {
      const yaml = ['id: onboard-sam', 'name: Onboard Sam', 'steps: []'].join(
        '\n'
      )
      Object.assign(commands, {
        draftWorkflowFromRequest: ok(yaml),
        createWorkflow: ok({ id: 'onboard-sam', name: 'Onboard Sam' }),
      })
      const user = userEvent.setup()
      show(local({ status: 'done', endedAt: Date.now() }))
      await user.click(
        await screen.findByRole('button', { name: 'Save as workflow' })
      )
      expect(
        await screen.findByRole('textbox', { name: 'Workflow file' })
      ).toHaveValue(yaml)
      expect(mocks().createWorkflow).not.toHaveBeenCalled()
      await user.click(screen.getByRole('button', { name: 'Save workflow' }))
      await waitFor(() =>
        expect(mocks().createWorkflow).toHaveBeenCalledWith({ yaml })
      )
      await waitFor(() =>
        expect(useNavigationStore.getState().current).toEqual({
          kind: 'workflow',
          workflowId: 'onboard-sam',
        })
      )
    })
  })
})
