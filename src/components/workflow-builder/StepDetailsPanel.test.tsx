import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT, type Workflow } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { StepDetailsPanel } from './StepDetailsPanel'
import { mockBuilderCommands, sampleWorkflow } from './test-support'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const nandi = { ...displayAgent(DEFAULT_AGENT, 'Nandi'), kind: 'default' }

/** `daily-report` with its research step sent to `agent` */
function workflowWithResearchBy(agent: string): Workflow {
  const workflow = sampleWorkflow()
  return {
    ...workflow,
    steps: workflow.steps.map(step =>
      step.id === 'research' ? { ...step, agent } : step
    ),
  }
}

describe('StepDetailsPanel agent picker', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of ['listAgents', 'getWorkflow', 'getWorkflowGraph'])
      original[name] = mocked[name]
    mockBuilderCommands()
    mocked.listAgents = vi
      .fn()
      .mockResolvedValue(
        ok([
          nandi,
          displayAgent('research-agent', 'Research Agent'),
          displayAgent('writer', 'Writer'),
        ])
      )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  const picker = async () => {
    const select = await screen.findByRole('combobox', { name: 'Agent' })
    await waitFor(() =>
      expect(within(select).getAllByRole('option')).toHaveLength(3)
    )
    return select
  }

  it('offers Nandi as a fixed choice above the person’s agents', async () => {
    mocked.getWorkflow = vi
      .fn()
      .mockResolvedValue(ok(workflowWithResearchBy('research-agent')))
    render(<StepDetailsPanel workflowId="daily-report" stepId="research" />)

    const select = await picker()
    const options = within(select).getAllByRole('option')
    expect(options[0]).toHaveValue(DEFAULT_AGENT)
    expect(options[0]).toHaveTextContent('Nandi')
    const group = within(select).getByRole('group', { name: 'Agents' })
    expect(
      within(group)
        .getAllByRole('option')
        .map(option => option.textContent)
    ).toEqual(['Research Agent', 'Writer'])
    expect(select).toHaveValue('research-agent')
  })

  it('knows a step sent to Nandi, so it is never "not installed"', async () => {
    mocked.getWorkflow = vi
      .fn()
      .mockResolvedValue(ok(workflowWithResearchBy(DEFAULT_AGENT)))
    render(<StepDetailsPanel workflowId="daily-report" stepId="research" />)

    const select = await picker()
    expect(select).toHaveValue(DEFAULT_AGENT)
    expect(within(select).queryByText(/not installed/)).toBeNull()
  })
})
