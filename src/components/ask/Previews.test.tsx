import { beforeEach, describe, expect, it, vi } from 'vitest'
import { streamAsksThroughAsk } from '@/test/ask-stream'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { ActionPlan, AskContext, EditPreview } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { ActionPlanPreview } from './ActionPlanPreview'
import { AskBox } from './AskBox'
import { EditPreviewCard } from './EditPreview'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    ask: vi.fn(),
    startRequest: vi.fn(),
    supports: { attachments: false },
    subscribe: vi.fn(() => ({
      ready: Promise.resolve(),
      unsubscribe: vi.fn(),
    })),
  },
}))

vi.mock('@/lib/client', () => ({
  client: mockClient,
  isDesktop: () => false,
}))

const HOME: AskContext = { currentView: 'home', focusedArtifact: null }

const plan: ActionPlan = {
  steps: [
    {
      kind: 'capability',
      target: 'code-review.python',
      description: 'Delegate to Python review (Alice)',
    },
    {
      kind: 'localAgent',
      target: 'writer',
      description: 'Send to Writer',
    },
  ],
  estimatedDurationSeconds: 420,
  estimatedCostUsd: 0.5,
  warnings: ['This runs 2 capabilities in order, as a new workflow'],
  rejection: null,
}

const preview: EditPreview = {
  artifact: { kind: 'capability', id: 'code-review.python' },
  before: 'sla_seconds: 300\n',
  after: 'sla_seconds: 60\n',
  diff: 'diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-  sla_seconds: 300\n+  sla_seconds: 60\n',
  impact: ['2 subscribers must pick up the new version: Reviewer, Alice'],
  versionBump: 'minor',
}

// The ask box streams; answers come from the `ask` mock
streamAsksThroughAsk(mockClient)

describe('ActionPlanPreview', () => {
  it('lists the steps, estimates, and warnings', () => {
    render(<ActionPlanPreview plan={plan} onApprove={vi.fn()} />)
    const steps = screen.getByRole('list', { name: /plan steps/i })
    expect(steps.querySelectorAll('li')).toHaveLength(2)
    expect(steps).toHaveTextContent('Delegate to Python review (Alice)')
    expect(steps).toHaveTextContent('code-review.python')
    expect(screen.getByText(/About 7m/)).toBeInTheDocument()
    expect(screen.getByText(/as a new workflow/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /run plan/i })).toBeEnabled()
  })

  it('shows why a plan was rejected', () => {
    render(
      <ActionPlanPreview
        plan={{ ...plan, steps: [], rejection: 'Nothing books flights' }}
      />
    )
    expect(screen.getByText('Nothing books flights')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })
})

describe('EditPreviewCard', () => {
  it('shows the diff, the bump, and the impact', async () => {
    const onApply = vi.fn()
    render(<EditPreviewCard preview={preview} onApply={onApply} />)
    expect(
      screen.getByText(/Capability code-review\.python/)
    ).toBeInTheDocument()
    expect(screen.getByText(/minor version bump/)).toBeInTheDocument()
    const diff = screen.getByLabelText(/changes/i)
    expect(diff).toHaveTextContent(/\+\s+sla_seconds: 60/)
    expect(diff).not.toHaveTextContent('diff --git')
    expect(screen.getByText(/Reviewer, Alice/)).toBeInTheDocument()
    await userEvent
      .setup()
      .click(screen.getByRole('button', { name: /apply/i }))
    expect(onApply).toHaveBeenCalled()
  })
})

describe('AskBox actions and edits', () => {
  beforeEach(() => {
    mockClient.ask.mockReset()
    mockClient.startRequest.mockReset()
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('saves an approved plan as a Request and opens it', async () => {
    mockClient.ask.mockResolvedValue({
      status: 'ok',
      data: { kind: 'action', data: plan },
    })
    mockClient.startRequest.mockResolvedValue({ status: 'ok', data: 'r1' })
    const user = userEvent.setup()
    render(<AskBox context={HOME} placeholder="Ask" />)
    await user.type(
      screen.getByRole('textbox'),
      'Review and write it up{Enter}'
    )

    // It delegates and messages an agent, so it waits to be approved
    expect(
      await screen.findByRole('button', { name: /run plan/i })
    ).toBeInTheDocument()
    expect(mockClient.startRequest).not.toHaveBeenCalled()

    await user.click(screen.getByRole('button', { name: /run plan/i }))
    await waitFor(() =>
      expect(mockClient.startRequest).toHaveBeenCalledWith(
        plan,
        'Review and write it up'
      )
    )
    await waitFor(() =>
      expect(useNavigationStore.getState().current).toEqual({
        kind: 'request',
        requestId: 'r1',
      })
    )
    await waitFor(() =>
      expect(screen.queryByText(/plan steps/i)).not.toBeInTheDocument()
    )
  })

  it('starts a plan that only involves people without asking', async () => {
    const peopleOnly: ActionPlan = {
      ...plan,
      steps: [
        { kind: 'human', target: 'inst-alice', description: 'Ask Alice' },
      ],
    }
    mockClient.ask.mockResolvedValue({
      status: 'ok',
      data: { kind: 'action', data: peopleOnly },
    })
    mockClient.startRequest.mockResolvedValue({ status: 'ok', data: 'r2' })
    const user = userEvent.setup()
    render(<AskBox context={HOME} placeholder="Ask" />)
    await user.type(screen.getByRole('textbox'), 'Ask Alice{Enter}')
    await waitFor(() =>
      expect(mockClient.startRequest).toHaveBeenCalledWith(
        peopleOnly,
        'Ask Alice'
      )
    )
    await waitFor(() =>
      expect(useNavigationStore.getState().current).toEqual({
        kind: 'request',
        requestId: 'r2',
      })
    )
  })

  it('hands an approved edit to its owner', async () => {
    mockClient.ask.mockResolvedValue({
      status: 'ok',
      data: { kind: 'edit', data: preview },
    })
    const onApplyEdit = vi.fn()
    const user = userEvent.setup()
    render(
      <AskBox context={HOME} placeholder="Ask" onApplyEdit={onApplyEdit} />
    )
    await user.type(screen.getByRole('textbox'), 'Make the SLA 60s{Enter}')
    await user.click(await screen.findByRole('button', { name: /apply/i }))
    expect(onApplyEdit).toHaveBeenCalledWith(preview)
  })
})
