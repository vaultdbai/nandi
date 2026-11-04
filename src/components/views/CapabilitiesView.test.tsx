import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import {
  listing,
  providerStats,
} from '@/components/capability/capability-fixtures'
import { CapabilityCard } from '@/components/capability/CapabilityCard'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { CapabilitiesView } from './CapabilitiesView'

const owned = listing(
  'code-review.python',
  { owned: true, provided: true, localProviders: 1, onlineProviders: 1 },
  { name: 'Python code review', tags: ['python'] }
)
const used = listing(
  'summarize-paper',
  { used: true, onlineProviders: 2, successRate: 0.9, invocations: 12 },
  {
    name: 'Summarize paper',
    owner: { kind: 'user', id: 'alice', displayName: 'Alice' },
  }
)
const discover = listing(
  'translate.ja',
  { source: 'network', onlineProviders: 1 },
  {
    name: 'Translate to Japanese',
    owner: { kind: 'user', id: 'bob', displayName: 'Bob' },
  }
)
const ownedOnly = listing(
  'draft-only',
  { owned: true },
  { name: 'Draft only', description: 'Not served yet' }
)

const CAPABILITIES_VIEW: View = {
  kind: 'network',
  tab: 'sharing',
  sharing: 'capabilities',
}

describe('CapabilitiesView', () => {
  beforeEach(() => {
    useNavigationStore.setState({
      current: CAPABILITIES_VIEW,
      history: [],
    })
    Object.assign(commands, {
      listCapabilities: ok([owned, used, discover, ownedOnly]),
      getCapabilityProviderBreakdown: ok([
        providerStats('reviewer', {
          load: 1,
          evidence: { count: 4, successCount: 3 },
        }),
      ]),
    })
  })

  it('renders the four relationship sections (tools live in the Library)', async () => {
    render(<CapabilitiesView />)
    expect(screen.queryByTestId('section-tools')).not.toBeInTheDocument()

    const ownedSection = within(await screen.findByTestId('section-owned'))
    expect(
      await ownedSection.findByText('Python code review')
    ).toBeInTheDocument()
    expect(ownedSection.getByText('Draft only')).toBeInTheDocument()

    const provided = within(screen.getByTestId('section-provided'))
    expect(provided.getByText('Python code review')).toBeInTheDocument()
    expect(provided.queryByText('Draft only')).not.toBeInTheDocument()

    const usedSection = within(screen.getByTestId('section-used'))
    expect(usedSection.getByText('Summarize paper')).toBeInTheDocument()

    const discoverSection = within(screen.getByTestId('section-discover'))
    expect(
      discoverSection.getByText('Translate to Japanese')
    ).toBeInTheDocument()
    expect(discoverSection.getByText(/On the network/)).toBeInTheDocument()
  })

  it('shows a friendly empty state per section', async () => {
    Object.assign(commands, { listCapabilities: ok([used]) })
    render(<CapabilitiesView />)
    const ownedSection = within(await screen.findByTestId('section-owned'))
    expect(
      await ownedSection.findByText("You don't own any capabilities yet")
    ).toBeInTheDocument()
    expect(
      ownedSection.getByText(/\.nandi\/capabilities\//)
    ).toBeInTheDocument()
    expect(
      within(screen.getByTestId('section-provided')).getByText(
        /provides: \[capability-id\]/
      )
    ).toBeInTheDocument()
  })

  it('searches and filters by ownership', async () => {
    const user = userEvent.setup()
    render(<CapabilitiesView />)
    await screen.findAllByText('Python code review')

    await user.type(screen.getByRole('searchbox'), 'japanese')
    expect(screen.queryByText('Python code review')).not.toBeInTheDocument()
    expect(screen.getByText('Translate to Japanese')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    // A select, so "Mine" and "Others'" can't overlap
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Ownership' }),
      'others'
    )
    expect(screen.queryByText('Draft only')).not.toBeInTheDocument()
    expect(screen.getByText('Summarize paper')).toBeInTheDocument()
  })

  it('reports a load error', async () => {
    Object.assign(commands, {
      listCapabilities: () =>
        Promise.resolve({ status: 'error', error: 'boom' }),
    })
    render(<CapabilitiesView />)
    // In plain words; "boom" is behind Details
    expect(
      await screen.findByText(
        "Couldn't load: Something went wrong. Open Details for what happened."
      )
    ).toBeInTheDocument()
    expect(screen.queryByText(/boom/)).not.toBeInTheDocument()
  })
})

describe('CapabilityCard', () => {
  beforeEach(() => {
    useNavigationStore.setState({
      current: CAPABILITIES_VIEW,
      history: [],
    })
    Object.assign(commands, {
      getCapabilityProviderBreakdown: ok([
        providerStats('reviewer', {
          load: 1,
          evidence: { count: 4, successCount: 3 },
        }),
        providerStats('remote', { local: false, instanceId: 'i-alice' }),
      ]),
    })
  })

  it('shows both faces for a capability you provide and use', async () => {
    render(
      <CapabilityCard
        listing={listing(
          'cap',
          { provided: true, used: true, onlineProviders: 3, invocations: 7 },
          { name: 'Both' }
        )}
      />
    )
    const provider = within(screen.getByTestId('provider-face'))
    expect(await provider.findByText('1 online')).toBeInTheDocument()
    expect(provider.getByText('1 / 2')).toBeInTheDocument()
    expect(provider.getByText('reviewer')).toBeInTheDocument()
    expect(provider.getByText('75%')).toBeInTheDocument()
    expect(provider.queryByText('remote')).not.toBeInTheDocument()

    const consumer = within(screen.getByTestId('consumer-face'))
    expect(consumer.getByText('3')).toBeInTheDocument()
    expect(consumer.getByText('7')).toBeInTheDocument()
    expect(consumer.getByRole('button', { name: 'Delegate' })).toBeEnabled()
  })

  it('shows only the consumer face for something to discover', () => {
    render(<CapabilityCard listing={listing('cap', { onlineProviders: 0 })} />)
    expect(screen.queryByTestId('provider-face')).not.toBeInTheDocument()
    expect(
      within(screen.getByTestId('consumer-face')).getByRole('button', {
        name: 'Delegate',
      })
    ).toBeDisabled()
  })

  it('is compact with neither face', () => {
    render(
      <CapabilityCard
        listing={listing('cap', { owned: true }, { description: 'Mine' })}
      />
    )
    expect(screen.queryByTestId('provider-face')).not.toBeInTheDocument()
    expect(screen.queryByTestId('consumer-face')).not.toBeInTheDocument()
    expect(screen.getByText('Mine')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Details' })).toBeInTheDocument()
  })

  it('warns about missing tools', () => {
    render(
      <CapabilityCard
        listing={listing('cap', { missingTools: ['gmail.send_email'] })}
      />
    )
    expect(
      screen.getByText('Missing tools: gmail.send_email')
    ).toBeInTheDocument()
  })

  it('opens the detail view on click', async () => {
    const user = userEvent.setup()
    render(<CapabilityCard listing={listing('code-review.python')} />)
    await user.click(screen.getByTestId('capability-code-review.python'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'capability-detail',
      id: 'code-review.python',
    })
  })

  it('delegates without leaving the page, then opens the thread', async () => {
    const user = userEvent.setup()
    Object.assign(commands, { delegateTask: ok('task-42') })
    render(
      <CapabilityCard
        listing={listing('summarize', { onlineProviders: 1 }, { name: 'Sum' })}
      />
    )
    await user.click(screen.getByRole('button', { name: 'Delegate' }))
    expect(useNavigationStore.getState().current).toEqual(CAPABILITIES_VIEW)
    await user.type(screen.getByLabelText('Request'), 'Summarize arXiv 2401.1')
    await user.click(screen.getByRole('button', { name: 'Send' }))
    expect(commands.delegateTask).toHaveBeenCalledWith(
      'summarize',
      { text: 'Summarize arXiv 2401.1' },
      null
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'delegation-thread',
      taskId: 'task-42',
    })
  })
})
