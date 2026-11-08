import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { AgentPlaybooks } from '@/lib/bindings'
import type { AgentInfo } from '@/types/agent'
import { AgentPlaybooksTab } from './AgentPlaybooksTab'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const agent = { id: 'chief-of-staff' } as AgentInfo
const pay = {
  name: 'pay-a-bill',
  description: 'Pay a bill online',
  when: 'A bill is due',
  body: '1. Plan first.',
}

describe('AgentPlaybooksTab', () => {
  const names = [
    'repoAgentPlaybooks',
    'repoSaveAgentPlaybook',
    'repoDeleteAgentPlaybook',
  ]
  const originals = names.map(name => mocked[name])
  const show = (data: AgentPlaybooks) => {
    mocked.repoAgentPlaybooks = vi.fn().mockResolvedValue(ok(data))
    render(<AgentPlaybooksTab agent={agent} onDirtyChange={vi.fn()} />)
  }

  beforeEach(() => {
    mocked.repoSaveAgentPlaybook = vi.fn().mockResolvedValue(ok(pay))
    mocked.repoDeleteAgentPlaybook = vi.fn().mockResolvedValue(ok(null))
  })
  afterEach(() => {
    names.forEach((name, i) => (mocked[name] = originals[i]))
  })

  it('lists the playbooks and opens the first', async () => {
    show({ readOnly: false, playbooks: [pay], problems: [] })
    expect(await screen.findByText('Pay a bill online')).toBeInTheDocument()
    expect(screen.getByLabelText('Steps (Markdown)')).toHaveValue(
      '1. Plan first.'
    )
    expect(screen.getByText('3 of 900 words')).toBeInTheDocument()
  })

  it('saves an edit under its name', async () => {
    show({ readOnly: false, playbooks: [pay], problems: [] })
    const body = await screen.findByLabelText('Steps (Markdown)')
    await userEvent.type(body, ' Then pay.')
    await userEvent.click(screen.getByRole('button', { name: 'Save playbook' }))
    expect(mocked.repoSaveAgentPlaybook).toHaveBeenCalledWith(
      'chief-of-staff',
      { ...pay, body: '1. Plan first. Then pay.' },
      'pay-a-bill'
    )
  })

  it('adds a new one only when it is complete', async () => {
    show({ readOnly: false, playbooks: [pay], problems: [] })
    await userEvent.click(
      await screen.findByRole('button', { name: 'New playbook' })
    )
    const save = screen.getByRole('button', { name: 'Save playbook' })
    await userEvent.type(screen.getByLabelText('Name'), 'pay-a-bill')
    expect(
      screen.getByText('Another playbook has this name.')
    ).toBeInTheDocument()
    expect(save).toBeDisabled()
    await userEvent.clear(screen.getByLabelText('Name'))
    await userEvent.type(screen.getByLabelText('Name'), 'occasions')
    await userEvent.type(screen.getByLabelText('When to use it'), 'A birthday')
    await userEvent.type(
      screen.getByLabelText('What it covers'),
      'Birthday messages'
    )
    await userEvent.type(screen.getByLabelText('Steps (Markdown)'), 'Draft.')
    await userEvent.click(save)
    expect(mocked.repoSaveAgentPlaybook).toHaveBeenCalledWith(
      'chief-of-staff',
      {
        name: 'occasions',
        description: 'Birthday messages',
        when: 'A birthday',
        body: 'Draft.',
      },
      null
    )
  })

  it('deletes after asking', async () => {
    show({ readOnly: false, playbooks: [pay], problems: [] })
    await userEvent.click(await screen.findByRole('button', { name: 'Delete' }))
    expect(screen.getByText('Delete pay-a-bill?')).toBeInTheDocument()
    const buttons = screen.getAllByRole('button', { name: 'Delete' })
    await userEvent.click(buttons[buttons.length - 1] as HTMLElement)
    expect(mocked.repoDeleteAgentPlaybook).toHaveBeenCalledWith(
      'chief-of-staff',
      'pay-a-bill'
    )
  })

  it("shows an installed agent's playbooks read-only, and broken files", async () => {
    show({
      readOnly: true,
      playbooks: [pay],
      problems: [{ file: 'bad.md', error: 'sets `model:`' }],
    })
    expect(
      await screen.findByText(/its playbooks are read-only/)
    ).toBeInTheDocument()
    expect(screen.getByText('bad.md: sets `model:`')).toBeInTheDocument()
    expect(screen.getByLabelText('Steps (Markdown)')).toBeDisabled()
    expect(screen.queryByRole('button', { name: 'New playbook' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Delete' })).toBeNull()
  })
})
