import { beforeEach, describe, expect, it } from 'vitest'
import { useAgentStore } from '@/store/agent-store'
import { agentInfo, agentSnapshot, run } from '@/test/fixtures'
import { effectiveStatus } from './agent-status'

const turnOf = (agentId: string) =>
  useAgentStore.getState().current[agentId] ?? null

describe('effectiveStatus', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
  })

  it('follows the live turn', () => {
    const agent = agentInfo('writer', 'Writer')
    expect(effectiveStatus(agent, null)).toBe('idle')
    expect(effectiveStatus(agent, run('r1', 'writer'))).toBe('running')
    expect(
      effectiveStatus(
        agent,
        run('r1', 'writer', { status: 'awaitingApproval' })
      )
    ).toBe('waitingApproval')
    const broken = agentInfo('writer', 'Writer', { status: 'error' })
    expect(effectiveStatus(broken, run('r1', 'writer'))).toBe('error')
  })

  // A snapshot loaded mid-run (its tool container starting reloads it)
  // says `running`; the run ending must still turn the badge idle
  it('turns idle when the run ends after a snapshot taken mid-run', () => {
    const current = run('r1', 'writer')
    useAgentStore
      .getState()
      .setAgentSnapshot(
        agentSnapshot(
          agentInfo('writer', 'Writer', { status: 'running' }),
          'writer',
          { currentRun: current, currentTurn: current }
        ),
        'writer'
      )
    const agent = () => {
      const loaded = useAgentStore.getState().agents.writer?.agent
      if (!loaded) throw new Error('snapshot not loaded')
      return loaded
    }
    expect(effectiveStatus(agent(), turnOf('writer'))).toBe('running')

    useAgentStore.getState().applyConversationEvent({
      type: 'turnDone',
      payload: {
        agentId: 'writer',
        conversationId: 'agent:writer',
        runId: 'r1',
        status: 'completed',
        userMessageId: 'u1',
        finalMessageId: 'm1',
        error: null,
      },
    })

    expect(agent().status).toBe('running') // as of the snapshot
    expect(effectiveStatus(agent(), turnOf('writer'))).toBe('idle')
  })
})
