import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT, type WorkflowTemplate } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { TemplateDialog } from './NewWorkflowMenu'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const digest: WorkflowTemplate = {
  id: 'daily-digest',
  name: 'Daily digest',
  steps: [
    { id: 'gather', needsAgent: true, requiresApproval: false, manual: false },
  ],
}

describe('TemplateDialog agent picker', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of ['listAgents', 'listWorkflowTemplates'])
      original[name] = mocked[name]
    mocked.listWorkflowTemplates = vi.fn().mockResolvedValue(ok([digest]))
    mocked.listAgents = vi
      .fn()
      .mockResolvedValue(
        ok([
          { ...displayAgent(DEFAULT_AGENT, 'Nandi'), kind: 'default' },
          displayAgent('writer', 'Writer'),
        ])
      )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('offers Nandi as a fixed choice above the person’s agents', async () => {
    render(<TemplateDialog open onOpenChange={vi.fn()} />)
    const picker = await screen.findByRole('combobox', {
      name: 'Agent for “Gather”',
    })
    await waitFor(() =>
      expect(within(picker).getAllByRole('option')).toHaveLength(3)
    )
    const options = within(picker).getAllByRole('option')
    expect(options[0]).toHaveTextContent('Choose an agent')
    expect(options[1]).toHaveValue(DEFAULT_AGENT)
    expect(options[1]).toHaveTextContent('Nandi')
    const group = within(picker).getByRole('group', { name: 'Agents' })
    expect(
      within(group)
        .getAllByRole('option')
        .map(option => option.textContent)
    ).toEqual(['Writer'])
  })
})
