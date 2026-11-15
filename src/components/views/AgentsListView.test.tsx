import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import { render, screen } from '@/test/test-utils'
import { AgentsListView } from './AgentsListView'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

describe('AgentsListView', () => {
  const original: Commands = {}

  beforeEach(() => {
    original.listAgents = mocked.listAgents
    mocked.listAgents = () =>
      Promise.resolve(
        ok([
          { ...displayAgent(DEFAULT_AGENT, 'Default Nandi'), kind: 'default' },
          displayAgent('writer', 'Writer'),
        ])
      )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('lists the person’s own agents, never Nandi', async () => {
    render(<AgentsListView />)
    expect(await screen.findByText('Writer')).toBeInTheDocument()
    expect(screen.queryByText('Default Nandi')).toBeNull()
  })
})
