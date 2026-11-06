import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { MyNotesView } from './MyNotesView'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const mine = (agentsCanWrite = true) => ({
  path: 'C:/Users/dev/.nandi/notes',
  managed: true,
  agentsCanWrite,
  notes: [
    {
      name: 'deadlines.md',
      title: 'Deadlines',
      updatedAt: Date.now(),
      size: 40,
    },
    { name: 'runbooks/etl.md', title: null, updatedAt: Date.now(), size: 10 },
  ],
})

beforeEach(() => {
  Object.assign(commands, {
    getMyNotes: ok(mine()),
    readNote: ok(
      '# Deadlines\n\n| Due | What |\n| --- | --- |\n| 2026-10-30 | SOC 1 list |\n'
    ),
    saveNote: ok({
      name: 'deadlines.md',
      title: 'Deadlines',
      updatedAt: 1,
      size: 1,
    }),
    deleteNote: ok(null),
    setNotesAgentsCanWrite: ok(mine(false)),
    listAgents: ok([]),
  })
})

describe('My notes', () => {
  it('lists the notes by folder and shows the first one', async () => {
    render(<MyNotesView />)
    expect(
      await screen.findByRole('button', { name: 'Deadlines' })
    ).toHaveAttribute('aria-current', 'page')
    expect(screen.getByText('runbooks/')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'etl' })).toBeInTheDocument()
    expect(await screen.findByText('SOC 1 list')).toBeInTheDocument()
    expect(screen.getByText('C:/Users/dev/.nandi/notes')).toBeInTheDocument()
  })

  it('creates a note from a typed name', async () => {
    const user = userEvent.setup()
    render(<MyNotesView />)
    await user.click(await screen.findByRole('button', { name: 'New note' }))
    await user.type(
      screen.getByRole('textbox', { name: /Name, e.g./ }),
      'decisions{Enter}'
    )
    await waitFor(() =>
      expect(commands.saveNote).toHaveBeenCalledWith(
        'decisions.md',
        '# Decisions\n\n'
      )
    )
  })

  it('turns Chief of Staff keeping the notes off', async () => {
    const user = userEvent.setup()
    render(<MyNotesView />)
    const keep = await screen.findByRole('switch', {
      name: 'Let Chief of Staff keep them up to date',
    })
    expect(keep).toBeChecked()
    await user.click(keep)
    await waitFor(() =>
      expect(commands.setNotesAgentsCanWrite).toHaveBeenCalledWith(false)
    )
  })

  it('says who changed the open note', async () => {
    Object.assign(commands, {
      listRecentChanges: ok([
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
      ]),
    })
    render(<MyNotesView />)
    expect(
      await screen.findByText('Updated just now by Chief of Staff')
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeInTheDocument()
  })

  it('offers Chief of Staff when there are no notes', async () => {
    Object.assign(commands, {
      getMyNotes: ok({ ...mine(), notes: [] }),
      listAgents: ok([{ id: 'chief-of-staff', name: 'Chief of Staff' }]),
    })
    render(<MyNotesView />)
    expect(await screen.findByText('No notes yet')).toBeInTheDocument()
    expect(
      await screen.findByRole('button', { name: 'Open Chief of Staff' })
    ).toBeInTheDocument()
  })
})
