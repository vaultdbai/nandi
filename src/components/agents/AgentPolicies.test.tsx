import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { AgentPolicies as AgentPoliciesData } from '@/lib/bindings'
import { AgentPolicies } from './AgentPolicies'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

describe('AgentPolicies', () => {
  const original = mocked.agentPolicies
  const show = (data: AgentPoliciesData) => {
    mocked.agentPolicies = vi.fn().mockResolvedValue(ok(data))
    render(<AgentPolicies agentId="writer" />)
  }

  beforeEach(() => {
    mocked.agentPolicies = vi.fn()
  })
  afterEach(() => {
    mocked.agentPolicies = original
  })

  it('lists what each policy changed', async () => {
    show({
      policies: ['finance-safe', 'no-shell'],
      blocked: null,
      changes: [
        {
          item: 'tool shell',
          outcome: 'removed',
          detail: null,
          by: 'no-shell',
        },
        {
          item: 'folder reports',
          outcome: 'narrowed',
          detail: 'read only',
          by: 'finance-safe',
        },
        {
          item: 'tool write_file',
          outcome: 'mustAsk',
          detail: null,
          by: 'finance-safe',
        },
      ],
    })
    expect(
      await screen.findByText('Policies that apply: finance-safe, no-shell')
    ).toBeInTheDocument()
    const rows = screen.getAllByRole('listitem')
    expect(rows).toHaveLength(3)
    expect(rows[0]).toHaveTextContent('tool shell')
    expect(rows[0]).toHaveTextContent('Removed')
    expect(rows[0]).toHaveTextContent('by no-shell')
    expect(rows[1]).toHaveTextContent('Narrowed to read only')
    expect(rows[2]).toHaveTextContent('Asks first')
    expect(mocked.agentPolicies).toHaveBeenCalledWith('writer')
  })

  it('says when no policy applies', async () => {
    show({ policies: [], blocked: null, changes: [] })
    expect(await screen.findByText(/No policy applies/)).toBeInTheDocument()
  })

  it('says why the agent can’t run', async () => {
    show({
      policies: [],
      blocked: "The policy 'oops' has an error",
      changes: [],
    })
    expect(await screen.findByRole('alert')).toHaveTextContent(
      "This agent can't run: The policy 'oops' has an error"
    )
  })

  it('says when the policies change nothing it asks for', async () => {
    show({ policies: ['broad'], blocked: null, changes: [] })
    expect(await screen.findByText(/don't change anything/)).toBeInTheDocument()
  })
})
