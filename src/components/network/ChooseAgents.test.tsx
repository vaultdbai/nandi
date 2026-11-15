import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { render, screen, within } from '@/test/test-utils'
import { ChooseAgents } from './ReachabilityChecklist'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

describe('ChooseAgents', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of ['listAgents', 'getAgentDelegation'])
      original[name] = mocked[name]
    mocked.getAgentDelegation = vi
      .fn()
      .mockResolvedValue(ok({ acceptsDelegation: false }))
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  const open = async (agents: unknown[]) => {
    mocked.listAgents = vi.fn().mockResolvedValue(ok(agents))
    render(<ChooseAgents onNone={vi.fn()} />)
    await userEvent.click(
      screen.getByRole('button', { name: 'Choose agents…' })
    )
    return screen.getByRole('dialog')
  }

  it('puts Nandi on top, apart from the person’s agents', async () => {
    const dialog = await open([
      { ...displayAgent(DEFAULT_AGENT, 'Nandi'), kind: 'default' },
      displayAgent('writer', 'Writer'),
    ])
    const [fixed, own] = await within(dialog).findAllByRole('list')
    if (!fixed || !own) throw new Error('two lists')
    expect(within(fixed).getByText('Nandi')).toBeInTheDocument()
    expect(within(own).getByText('Writer')).toBeInTheDocument()
    expect(within(own).queryByText('Nandi')).toBeNull()
  })

  it('offers Nandi even with no agents of the person’s own', async () => {
    const dialog = await open([
      { ...displayAgent(DEFAULT_AGENT, 'Nandi'), kind: 'default' },
    ])
    expect(await within(dialog).findByText('Nandi')).toBeInTheDocument()
    expect(within(dialog).getByText('You have no agents yet.')).toBeVisible()
  })
})
