import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { commands } from '@/lib/tauri-bindings'
import type { ShippedUpdate } from '@/lib/tauri-bindings'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { NandiUpdateCard } from './NandiUpdateCard'
import { shippedFileName } from './nandi-update-model'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const ROLE: ShippedUpdate = {
  path: '.nandi/agents/nandi/instructions.md',
  mine: '# My role\n- **Name and title:** Ada\n',
  shipped: '# My role\n- **Name and title:**\n- **Working hours:**\n',
  diff: [
    '--- a/.nandi/agents/nandi/instructions.md',
    '+++ b/.nandi/agents/nandi/instructions.md',
    '@@ -1,2 +1,3 @@',
    ' # My role',
    '-- **Name and title:** Ada',
    '+- **Name and title:**',
    '+- **Working hours:**',
  ].join('\n'),
}

describe('shippedFileName', () => {
  it('names a file the way the person knows it', () => {
    const cases: [path: string, name: string][] = [
      ['.nandi/agents/nandi/instructions.md', 'instructions.md'],
      ['.nandi/agents/nandi/skills/occasions.md', 'skills/occasions.md'],
      ['.nandi/triggers/mail-check.yaml', 'triggers/mail-check.yaml'],
      ['elsewhere.md', 'elsewhere.md'],
    ]
    for (const [path, name] of cases) expect(shippedFileName(path)).toBe(name)
  })
})

describe('NandiUpdateCard', () => {
  const original: Commands = {}
  let take: ReturnType<typeof vi.fn>
  let keep: ReturnType<typeof vi.fn>

  const offer = (updates: ShippedUpdate[]) => {
    mocked.defaultAgentUpdates = vi.fn().mockResolvedValue(ok(updates))
  }

  beforeEach(() => {
    for (const name of [
      'defaultAgentUpdates',
      'takeDefaultAgentUpdate',
      'keepDefaultAgentFile',
    ])
      original[name] = mocked[name]
    take = vi.fn().mockResolvedValue(ok(null))
    keep = vi.fn().mockResolvedValue(ok(null))
    mocked.takeDefaultAgentUpdate = take
    mocked.keepDefaultAgentFile = keep
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  const renderCard = () => render(<NandiUpdateCard agent={{ name: 'Nandi' }} />)

  it('shows nothing without an update', async () => {
    offer([])
    renderCard()
    await waitFor(() => expect(mocked.defaultAgentUpdates).toHaveBeenCalled())
    expect(screen.queryByRole('heading')).not.toBeInTheDocument()
  })

  it('says an update is available and shows the diff from yours', async () => {
    offer([ROLE])
    renderCard()
    expect(
      await screen.findByRole('heading', {
        name: "An update to Nandi's instructions is available",
      })
    ).toBeInTheDocument()
    const item = screen.getByRole('listitem', { name: 'instructions.md' })
    expect(within(item).getByText('-- **Name and title:** Ada')).toBeVisible()
    expect(within(item).getByText('+- **Working hours:**')).toBeVisible()
    expect(within(item).queryByText(/^\+\+\+ /)).not.toBeInTheDocument()
  })

  it('takes the update for that file', async () => {
    const user = userEvent.setup()
    offer([ROLE])
    renderCard()
    await user.click(await screen.findByRole('button', { name: 'Take update' }))
    await waitFor(() => expect(take).toHaveBeenCalledWith(ROLE.path))
    expect(keep).not.toHaveBeenCalled()
  })

  it('keeps the person’s file', async () => {
    const user = userEvent.setup()
    offer([ROLE])
    renderCard()
    await user.click(await screen.findByRole('button', { name: 'Keep mine' }))
    await waitFor(() => expect(keep).toHaveBeenCalledWith(ROLE.path))
    expect(take).not.toHaveBeenCalled()
  })

  it('offers each file on its own', async () => {
    const user = userEvent.setup()
    const playbook = {
      ...ROLE,
      path: '.nandi/agents/nandi/skills/occasions.md',
    }
    offer([ROLE, playbook])
    renderCard()
    const item = await screen.findByRole('listitem', {
      name: 'skills/occasions.md',
    })
    expect(screen.getAllByRole('listitem')).toHaveLength(2)
    await user.click(within(item).getByRole('button', { name: 'Take update' }))
    await waitFor(() => expect(take).toHaveBeenCalledWith(playbook.path))
  })

  it('checks for updates again after a choice', async () => {
    const user = userEvent.setup()
    offer([ROLE])
    renderCard()
    await user.click(await screen.findByRole('button', { name: 'Keep mine' }))
    await waitFor(() =>
      expect(mocked.defaultAgentUpdates).toHaveBeenCalledTimes(2)
    )
  })
})
