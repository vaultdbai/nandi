import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import {
  cluster,
  definition,
  evidence,
  listing,
  networkMember,
  providerStats,
} from '@/components/capability/capability-fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { CapabilityDetailView } from './CapabilityDetailView'

const ID = 'code-review.python'

const def = definition(ID, {
  name: 'Python code review',
  description: 'Reviews Python pull requests',
  body: '## How it works\n\nRuns the linters.',
  requiredTools: ['builtin.shell', 'github.get_pr'],
  tags: ['python'],
  schemas: {
    input: { type: 'object', properties: { repo: { type: 'string' } } },
    output: null,
  },
})

const promoted = definition(ID, {
  ...def,
  schemas: {
    input: {
      type: 'object',
      properties: {
        repo: { type: 'string' },
        python_version: { type: 'string' },
      },
    },
    output: null,
  },
})

function setup(owned = true) {
  useNavigationStore.setState({
    current: { kind: 'capability-detail', id: ID },
    history: [],
  })
  Object.assign(commands, {
    getCapability: ok(def),
    listCapabilities: ok([listing(ID, { owned, provided: true })]),
    getCapabilityProviderBreakdown: ok([
      providerStats('reviewer', {
        evidence: { count: 10, successCount: 9, p50Ms: 1200 },
      }),
      providerStats('review-flow', {
        kind: 'workflow',
        evidence: { count: 2, successCount: 2, p50Ms: 4000 },
      }),
      providerStats('alice-reviewer', {
        local: false,
        instanceId: 'i-alice',
        online: false,
      }),
    ]),
    getCapabilityEvidence: ok([
      evidence({ count: 10, successCount: 9, p50Ms: 1000, costUsdTotal: 1.5 }),
    ]),
    listCapabilityAmbiguities: ok([
      cluster('c-small', { clusterLabel: 'Which branch?', occurrenceCount: 3 }),
      cluster('c-python'),
    ]),
    promoteAmbiguityToSchema: ok(promoted),
    getNetworkSnapshot: ok({
      relayUrl: null,
      relayConnected: true,
      members: [
        networkMember('i-me', 'Me', 'me'),
        networkMember('i-alice', 'Alice', 'alice'),
      ],
      fetchedAt: 0,
    }),
    transferCapabilityOwnership: ok(def),
    saveCapability: ok(def),
  })
}

describe('CapabilityDetailView', () => {
  beforeEach(() => setup())

  it('renders the overview: description, body, tools, tags, schema', async () => {
    render(<CapabilityDetailView capabilityId={ID} />)
    expect(
      await screen.findByRole('heading', { name: 'Python code review' })
    ).toBeInTheDocument()
    expect(screen.getByText('Reviews Python pull requests')).toBeInTheDocument()
    expect(screen.getByText('How it works')).toBeInTheDocument()
    expect(screen.getByText('github.get_pr')).toBeInTheDocument()
    expect(screen.getByText('python')).toBeInTheDocument()
    expect(screen.getByText(/"repo"/)).toBeInTheDocument()
    expect(screen.getByText('No schema')).toBeInTheDocument()
  })

  it('renders every tab', async () => {
    const user = userEvent.setup()
    render(<CapabilityDetailView capabilityId={ID} />)
    await screen.findByRole('heading', { name: 'Python code review' })

    await user.click(screen.getByRole('tab', { name: 'Providers' }))
    expect(
      await screen.findByTestId('provider-row-reviewer')
    ).toBeInTheDocument()
    expect(screen.getByTestId('provider-row-review-flow')).toHaveTextContent(
      'Workflow'
    )

    await user.click(screen.getByRole('tab', { name: 'Evidence' }))
    const panel = within(await screen.findByTestId('evidence-panel'))
    expect(panel.getByText('90%')).toBeInTheDocument()
    expect(panel.getByText('$1.50')).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Ambiguities' }))
    const items = await screen.findAllByTestId(/^ambiguity-/)
    // Most frequent first
    expect(items[0]).toHaveTextContent('Which Python version?')
    expect(items[1]).toHaveTextContent('Which branch?')

    await user.click(screen.getByRole('tab', { name: 'Policy' }))
    expect(await screen.findByLabelText('Visibility')).toHaveValue('network')
    expect(
      screen.getByRole('button', { name: 'Save policy' })
    ).toBeInTheDocument()
  })

  it('sorts the provider table and opens a provider', async () => {
    const user = userEvent.setup()
    render(<CapabilityDetailView capabilityId={ID} />)
    await user.click(await screen.findByRole('tab', { name: 'Providers' }))
    await screen.findByTestId('provider-row-reviewer')

    const order = () =>
      screen.getAllByTestId(/^provider-row-/).map(row => row.dataset.testid)
    // Numbers sort slowest first; unknown last either way
    await user.click(screen.getByRole('button', { name: 'p50' }))
    expect(order()).toEqual([
      'provider-row-review-flow',
      'provider-row-reviewer',
      'provider-row-alice-reviewer',
    ])
    await user.click(screen.getByRole('button', { name: 'p50' }))
    expect(order()).toEqual([
      'provider-row-reviewer',
      'provider-row-review-flow',
      'provider-row-alice-reviewer',
    ])

    await user.click(screen.getByTestId('provider-row-review-flow'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'workflow',
      workflowId: 'review-flow',
    })
  })

  it('promotes an ambiguity to the input schema', async () => {
    const user = userEvent.setup()
    render(<CapabilityDetailView capabilityId={ID} />)
    await user.click(await screen.findByRole('tab', { name: 'Ambiguities' }))
    const item = within(await screen.findByTestId('ambiguity-c-python'))
    await user.click(
      item.getByRole('button', { name: 'Promote to input schema' })
    )
    expect(commands.promoteAmbiguityToSchema).toHaveBeenCalledWith(
      ID,
      'c-python'
    )
    await user.click(screen.getByRole('tab', { name: 'Overview' }))
    expect(await screen.findByText(/"python_version"/)).toBeInTheDocument()
  })

  it('transfers ownership after confirming', async () => {
    const user = userEvent.setup()
    render(<CapabilityDetailView capabilityId={ID} />)
    await user.click(await screen.findByRole('button', { name: 'Transfer' }))

    const dialog = within(await screen.findByRole('dialog'))
    const select = dialog.getByLabelText('New owner')
    // The current owner isn't offered
    await waitFor(() =>
      expect(within(select).getAllByRole('option')).toHaveLength(2)
    )
    await user.selectOptions(select, 'i-alice')
    await user.type(dialog.getByLabelText('Reason (optional)'), 'Team move')
    await user.click(dialog.getByRole('button', { name: 'Continue' }))

    expect(dialog.getByRole('alert')).toHaveTextContent('Transfer to Alice?')
    expect(commands.transferCapabilityOwnership).not.toHaveBeenCalled()
    await user.click(dialog.getByRole('button', { name: 'Transfer ownership' }))

    expect(commands.transferCapabilityOwnership).toHaveBeenCalledWith(
      ID,
      { kind: 'user', id: 'alice', displayName: 'Alice', transferable: true },
      'Team move'
    )
  })

  it('saves the policy as the owner', async () => {
    const user = userEvent.setup()
    render(<CapabilityDetailView capabilityId={ID} />)
    await user.click(await screen.findByRole('tab', { name: 'Policy' }))
    await user.selectOptions(await screen.findByLabelText('Visibility'), 'team')
    await user.click(screen.getByLabelText('Requests need approval'))
    await user.click(screen.getByRole('button', { name: 'Save policy' }))
    expect(commands.saveCapability).toHaveBeenCalledWith(
      expect.objectContaining({
        id: ID,
        policy: expect.objectContaining({
          visibility: 'team',
          approvalRequired: true,
        }),
      })
    )
  })

  it('is read-only for someone else’s capability', async () => {
    setup(false)
    const user = userEvent.setup()
    render(<CapabilityDetailView capabilityId={ID} />)
    await screen.findByRole('heading', { name: 'Python code review' })
    expect(
      screen.queryByRole('button', { name: 'Transfer' })
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: 'Edit' })
    ).not.toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Policy' }))
    expect(
      await screen.findByText('Only the owner can change the policy.')
    ).toBeInTheDocument()
    expect(screen.getByLabelText('Visibility')).toBeDisabled()

    await user.click(screen.getByRole('tab', { name: 'Ambiguities' }))
    await screen.findByTestId('ambiguity-c-python')
    expect(
      screen.queryByRole('button', { name: 'Promote to input schema' })
    ).not.toBeInTheDocument()
  })
})
