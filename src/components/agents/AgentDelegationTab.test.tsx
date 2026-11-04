import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { ClarificationPolicy } from '@/lib/bindings'
import { AgentDelegationTab } from './AgentDelegationTab'

const saved: ClarificationPolicy = {
  allow: true,
  maxRounds: 3,
  autoTopics: ['formatting'],
  escalateTopics: [],
  timeoutSeconds: 120,
}

describe('AgentDelegationTab', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getClarificationPolicy: vi
        .fn()
        .mockResolvedValue({ status: 'ok', data: saved }),
      setClarificationPolicy: vi
        .fn()
        .mockImplementation(async (_id: string, p: ClarificationPolicy) => ({
          status: 'ok',
          data: p,
        })),
    })
  })

  it("loads the agent's policy and saves edits", async () => {
    render(<AgentDelegationTab agentId="writer" />)
    const timeout = await screen.findByLabelText('Answer within (seconds)')
    expect(timeout).toHaveValue(120)
    expect(commands.getClarificationPolicy).toHaveBeenCalledWith('writer')

    const save = screen.getByRole('button', { name: 'Save policy' })
    expect(save).toBeDisabled()
    fireEvent.change(timeout, { target: { value: '600' } })
    expect(save).toBeEnabled()
    fireEvent.click(save)

    await waitFor(() =>
      expect(commands.setClarificationPolicy).toHaveBeenCalledWith('writer', {
        ...saved,
        timeoutSeconds: 600,
      })
    )
    expect(await screen.findByRole('status')).toHaveTextContent(
      "Saved to the agent's skill.md"
    )
    expect(screen.getByRole('button', { name: 'Save policy' })).toBeDisabled()
  })

  it('shows an error when saving fails', async () => {
    Object.assign(commands, {
      setClarificationPolicy: vi
        .fn()
        .mockResolvedValue({ status: 'error', error: 'disk full' }),
    })
    render(<AgentDelegationTab agentId="writer" />)
    const timeout = await screen.findByLabelText('Answer within (seconds)')
    fireEvent.change(timeout, { target: { value: '600' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save policy' }))
    expect(await screen.findByText('disk full')).toBeInTheDocument()
    expect(
      screen.getByText("Couldn't save the clarification policy")
    ).toBeInTheDocument()
  })

  it('shows an error when the policy cannot load', async () => {
    Object.assign(commands, {
      getClarificationPolicy: vi
        .fn()
        .mockResolvedValue({ status: 'error', error: 'no such agent' }),
    })
    render(<AgentDelegationTab agentId="writer" />)
    expect(await screen.findByText('no such agent')).toBeInTheDocument()
  })
})
