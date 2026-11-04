import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import type { MemoryEntry } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { AgentMemoryPanel } from './AgentMemoryPanel'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    loadAgentMemory: vi.fn(),
    updateAgentMemory: vi.fn(),
    deleteAgentMemory: vi.fn(),
    refreshAgentMemory: vi.fn(),
    getAgentMemoryEnabled: vi.fn(),
    setAgentMemoryEnabled: vi.fn(),
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

const ok = <T,>(data: T) => Promise.resolve({ status: 'ok' as const, data })

const OLDER: MemoryEntry = {
  content: 'Reports go in out/.',
  createdAt: Date.UTC(2026, 9, 11, 9, 0),
  sourceRun: null,
}
const NEWER: MemoryEntry = {
  content: 'The user prefers concise summaries with bullet points.',
  createdAt: Date.UTC(2026, 9, 12, 14, 30),
  sourceRun: 'abc12345-run',
}

describe('AgentMemoryPanel', () => {
  // The file: what the next load returns
  let memory: MemoryEntry[]
  let enabled: boolean

  beforeEach(() => {
    vi.clearAllMocks()
    memory = [OLDER, NEWER]
    enabled = true
    mockClient.loadAgentMemory.mockImplementation(() => ok(memory))
    mockClient.getAgentMemoryEnabled.mockImplementation(() => ok(enabled))
    mockClient.updateAgentMemory.mockImplementation(
      (_agentId: string, index: number, content: string) => {
        memory = memory.map((entry, i) =>
          i === index ? { ...entry, content } : entry
        )
        return ok(null)
      }
    )
    mockClient.deleteAgentMemory.mockImplementation(
      (_agentId: string, index: number) => {
        memory = memory.filter((_, i) => i !== index)
        return ok(null)
      }
    )
    mockClient.setAgentMemoryEnabled.mockImplementation(
      (_agentId: string, value: boolean) => {
        enabled = value
        return ok(null)
      }
    )
    mockClient.refreshAgentMemory.mockImplementation(() => {
      memory = [
        ...memory,
        { content: 'Uses pnpm.', createdAt: Date.now(), sourceRun: 'r9' },
      ]
      return ok(memory)
    })
  })

  it('lists entries newest first with their source run', async () => {
    const navigate = vi.spyOn(useNavigationStore.getState(), 'navigate')
    const user = userEvent.setup()
    render(<AgentMemoryPanel agentId="writer" />)

    const list = await screen.findByRole('list', { name: 'Memory entries' })
    const items = within(list).getAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(items[0]).toHaveTextContent(NEWER.content)
    expect(items[1]).toHaveTextContent(OLDER.content)
    expect(mockClient.loadAgentMemory).toHaveBeenCalledWith('writer')

    await user.click(within(items[0] as HTMLElement).getByText('Run abc12345'))
    expect(navigate).toHaveBeenCalledWith({
      kind: 'run-detail',
      runId: 'abc12345-run',
    })
    navigate.mockRestore()
  })

  it('shows the empty state', async () => {
    memory = []
    render(<AgentMemoryPanel agentId="writer" />)
    expect(
      await screen.findByText(
        'This agent has no memory yet. It will write here as it learns about your preferences.'
      )
    ).toBeInTheDocument()
  })

  it('edits an entry in place', async () => {
    const user = userEvent.setup()
    render(<AgentMemoryPanel agentId="writer" />)
    const list = await screen.findByRole('list', { name: 'Memory entries' })
    const newest = within(list).getAllByRole('listitem')[0] as HTMLElement

    await user.click(within(newest).getByRole('button', { name: 'Edit entry' }))
    const box = screen.getByRole('textbox', { name: 'Memory entry' })
    await user.clear(box)
    await user.type(box, 'Prefers tables.')
    await user.click(screen.getByRole('button', { name: 'Save' }))

    // The newest is index 1 in the file
    expect(mockClient.updateAgentMemory).toHaveBeenCalledWith(
      'writer',
      1,
      'Prefers tables.'
    )
    expect(await screen.findByText('Prefers tables.')).toBeInTheDocument()
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
  })

  it('deletes an entry', async () => {
    const user = userEvent.setup()
    render(<AgentMemoryPanel agentId="writer" />)
    const list = await screen.findByRole('list', { name: 'Memory entries' })
    const oldest = within(list).getAllByRole('listitem')[1] as HTMLElement

    await user.click(
      within(oldest).getByRole('button', { name: 'Delete entry' })
    )
    expect(mockClient.deleteAgentMemory).toHaveBeenCalledWith('writer', 0)
    await waitFor(() =>
      expect(screen.queryByText(OLDER.content)).not.toBeInTheDocument()
    )
    expect(screen.getByText(NEWER.content)).toBeInTheDocument()
  })

  it('refreshes from the latest session and toggles memory off', async () => {
    const user = userEvent.setup()
    render(<AgentMemoryPanel agentId="writer" />)
    await screen.findByRole('list', { name: 'Memory entries' })

    await user.click(
      screen.getByRole('button', { name: 'Refresh from session' })
    )
    expect(mockClient.refreshAgentMemory).toHaveBeenCalledWith('writer')
    expect(await screen.findByText('Uses pnpm.')).toBeInTheDocument()

    const toggle = screen.getByRole('switch', {
      name: 'Disable memory for this agent',
    })
    expect(toggle).not.toBeChecked()
    await user.click(toggle)
    expect(mockClient.setAgentMemoryEnabled).toHaveBeenCalledWith(
      'writer',
      false
    )
    await waitFor(() => expect(toggle).toBeChecked())
    expect(
      screen.getByRole('button', { name: 'Refresh from session' })
    ).toBeDisabled()
  })
})
