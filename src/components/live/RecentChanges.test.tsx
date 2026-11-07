import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import type { LiveChange } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { RecentChanges } from './RecentChanges'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const changes: LiveChange[] = [
  {
    id: 'c1',
    area: 'notes',
    targetId: 'deadlines.md',
    label: 'deadlines.md',
    paths: ['deadlines.md'],
    pathCount: 1,
    action: 'updated',
    author: {
      kind: 'agent',
      agentId: 'chief-of-staff',
      agentName: 'Chief of Staff',
    },
    at: Date.now(),
  },
  {
    id: 'c2',
    area: 'work',
    targetId: 'analyst',
    label: 'Analyst',
    paths: ['report.csv'],
    pathCount: 1,
    action: 'created',
    author: {
      kind: 'workflow',
      workflowId: 'weekly',
      workflowName: 'Weekly report',
    },
    at: Date.now(),
  },
]

describe('Recent changes', () => {
  beforeEach(() => {
    Object.assign(commands, { listRecentChanges: ok(changes) })
  })

  it('says what changed and who changed it', async () => {
    render(<RecentChanges />)
    expect(await screen.findByText('Changed deadlines.md')).toBeInTheDocument()
    expect(screen.getByText('by Chief of Staff · just now')).toBeInTheDocument()
    expect(screen.getByText('Added Analyst')).toBeInTheDocument()
    expect(screen.getByText('report.csv')).toBeInTheDocument()
    expect(
      screen.getByText('by the Weekly report workflow · just now')
    ).toBeInTheDocument()
  })

  it('opens what changed', async () => {
    const user = userEvent.setup()
    const navigate = vi.spyOn(useNavigationStore.getState(), 'navigate')
    render(<RecentChanges />)
    await user.click(await screen.findByText('Changed deadlines.md'))
    expect(navigate).toHaveBeenCalledWith({
      kind: 'notes',
      note: 'deadlines.md',
    })
    navigate.mockRestore()
  })

  it('stays hidden until something changes', async () => {
    Object.assign(commands, { listRecentChanges: ok([]) })
    const { container } = render(<RecentChanges />)
    await new Promise(resolve => setTimeout(resolve, 0))
    expect(container).toBeEmptyDOMElement()
  })
})
