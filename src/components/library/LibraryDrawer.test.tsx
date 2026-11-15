import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT, type PilotDefinition } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { LibraryDrawer } from './LibraryDrawer'
import type { LibraryEntry } from './library-entry'
import { pilotItem } from './library-model'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const browser: PilotDefinition = {
  id: 'browser',
  name: 'Browser',
  platform: 'any',
  installCommand: '',
  startCommand: '',
  mcpPort: 0,
  description: 'Drives a browser you can watch',
  capabilities: [],
}

const entry: LibraryEntry = {
  item: pilotItem(browser, undefined, true),
  source: { type: 'pilot', pilot: browser, status: undefined },
}

describe('LibraryDrawer pilot grant', () => {
  const original: Commands = {}

  beforeEach(() => {
    original.listAgents = mocked.listAgents
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

  const picker = async () => {
    const select = await screen.findByRole('combobox', {
      name: 'Agent to allow Browser for',
    })
    await waitFor(() =>
      expect(within(select).getAllByRole('option')).toHaveLength(3)
    )
    return select
  }

  const renderDrawer = () =>
    render(
      <LibraryDrawer
        entry={entry}
        onClose={vi.fn()}
        onInstall={vi.fn()}
        onOpenDrawer={vi.fn()}
      />
    )

  it('offers Nandi as a fixed choice above the person’s agents', async () => {
    renderDrawer()
    const select = await picker()
    const options = within(select).getAllByRole('option')
    expect(options[0]).toHaveTextContent('Choose an agent')
    expect(options[1]).toHaveValue(DEFAULT_AGENT)
    expect(options[1]).toHaveTextContent('Nandi')
    const group = within(select).getByRole('group', { name: 'Agents' })
    expect(
      within(group)
        .getAllByRole('option')
        .map(option => option.textContent)
    ).toEqual(['Writer'])
  })

  it('lets Nandi be allowed the pilot', async () => {
    const user = userEvent.setup()
    renderDrawer()
    const allow = screen.getByRole('button', { name: /Allow/ })
    expect(allow).toBeDisabled()
    await user.selectOptions(await picker(), DEFAULT_AGENT)
    expect(allow).toBeEnabled()
  })
})
