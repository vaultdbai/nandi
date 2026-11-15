import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { emptyForm } from '@/lib/trigger-form'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { TriggerForm } from './TriggerForm'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

describe('TriggerForm target', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of ['listAgents', 'getWorkflows'])
      original[name] = mocked[name]
    mocked.listAgents = vi
      .fn()
      .mockResolvedValue(
        ok([
          { ...displayAgent(DEFAULT_AGENT, 'Nandi'), kind: 'default' },
          displayAgent('writer', 'Writer'),
        ])
      )
    mocked.getWorkflows = vi.fn().mockResolvedValue(ok([]))
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('offers Nandi as a fixed choice above the person’s agents', async () => {
    render(<TriggerForm form={emptyForm()} onChange={vi.fn()} />)
    const picker = screen.getByRole('combobox', { name: 'Run' })
    await waitFor(() =>
      expect(within(picker).getAllByRole('option')).toHaveLength(3)
    )
    const options = within(picker).getAllByRole('option')
    expect(options[1]).toHaveValue(DEFAULT_AGENT)
    const group = within(picker).getByRole('group', { name: 'Agents' })
    expect(
      within(group)
        .getAllByRole('option')
        .map(option => option.textContent)
    ).toEqual(['Writer'])
  })
})
