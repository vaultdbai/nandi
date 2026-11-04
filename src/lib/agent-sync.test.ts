import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { commands } from '@/lib/tauri-bindings'
import { useAgentStore } from '@/store/agent-store'
import { clearMockEventHandlers, emitMockEvent } from '@/test/mock-events'
import {
  T0,
  agentInfo,
  agentMessage,
  agentSnapshot,
  dashboardSnapshot,
  queueEntry,
  run,
  userMessage,
} from '@/test/fixtures'
import {
  handleEvent,
  handleNandiEvent,
  loadAgent,
  startAgentSync,
} from './agent-sync'
import { sendAgentMessage } from './agent-actions'
import { queryClient } from './query-client'

const WRITER = agentInfo('writer', 'Writer')
const ids = { agentId: 'writer', conversationId: 'agent:writer' }
const snapshotOk = (messages = [userMessage('u1', 'hi')]) => ({
  status: 'ok' as const,
  data: agentSnapshot(WRITER, 'writer', { messages }),
})

const messages = () =>
  useAgentStore.getState().conversations['agent:writer']?.messages ?? []

const queueIds = () =>
  (useAgentStore.getState().queue.writer ?? []).map(e => e.runId)

describe('agent-sync', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
    clearMockEventHandlers()
    vi.mocked(commands.getAgentSnapshot).mockResolvedValue(snapshotOk())
  })

  afterEach(() => vi.mocked(commands.getAgentSnapshot).mockReset())

  it('registers listeners once at boot, then loads every snapshot', async () => {
    vi.mocked(commands.listAgents).mockResolvedValueOnce({
      status: 'ok',
      data: [WRITER],
    })
    vi.mocked(commands.getDashboardSnapshot).mockResolvedValueOnce({
      status: 'ok',
      data: dashboardSnapshot({
        stats: {
          needsAttention: 2,
          runningAgents: 0,
          runsToday: 0,
          failedLastDay: 0,
        },
      }),
    })
    const stop = startAgentSync()

    await vi.waitFor(() =>
      expect(useAgentStore.getState().agents.writer).toBeDefined()
    )
    expect(useAgentStore.getState().dashboard?.stats.needsAttention).toBe(2)

    // Events now reach the store regardless of what the UI shows
    emitMockEvent('agent://token', {
      ...ids,
      runId: 'r1',
      messageId: 'm1',
      offset: 0,
      text: 'Hello',
      createdAt: T0,
    })
    expect(messages().at(-1)).toMatchObject({ kind: 'agent', content: 'Hello' })
    stop()
  })

  it('buffers events that arrive while a snapshot loads, then replays them', async () => {
    let resolve!: (value: ReturnType<typeof snapshotOk>) => void
    vi.mocked(commands.getAgentSnapshot).mockReturnValueOnce(
      new Promise(r => (resolve = r))
    )

    const loading = loadAgent('writer')
    // Emitted after the snapshot query ran, but before it returned
    handleEvent({
      type: 'token',
      payload: {
        ...ids,
        runId: 'r1',
        messageId: 'm1',
        offset: 0,
        text: 'Hi',
        createdAt: T0,
      },
    })
    expect(messages()).toEqual([])

    resolve(snapshotOk())
    await loading

    expect(messages().map(m => m.id)).toEqual(['u1', 'm1'])
  })

  it('replaying an event the snapshot already contains is harmless', async () => {
    let resolve!: (value: ReturnType<typeof snapshotOk>) => void
    vi.mocked(commands.getAgentSnapshot).mockReturnValueOnce(
      new Promise(r => (resolve = r))
    )

    const loading = loadAgent('writer')
    handleEvent({
      type: 'token',
      payload: {
        ...ids,
        runId: 'r1',
        messageId: 'm1',
        offset: 0,
        text: 'Hi',
        createdAt: T0,
      },
    })
    resolve(snapshotOk([userMessage('u1', 'hi'), agentMessage('m1', 'Hi')]))
    await loading

    expect(messages().map(m => [m.id, 'content' in m && m.content])).toEqual([
      ['u1', 'hi'],
      ['m1', 'Hi'],
    ])
  })

  it('reloads the snapshot when an event reveals a gap', async () => {
    await loadAgent('writer')
    vi.mocked(commands.getAgentSnapshot).mockClear()

    handleEvent({
      type: 'toolResult',
      payload: {
        ...ids,
        runId: 'r1',
        messageId: 'never-seen',
        result: 'x',
        isError: false,
      },
    })

    await vi.waitFor(() =>
      expect(commands.getAgentSnapshot).toHaveBeenCalledWith('writer')
    )
  })

  it('sends optimistically and confirms with the same id', async () => {
    await loadAgent('writer')
    let resolve!: (
      value: Awaited<ReturnType<typeof commands.sendMessage>>
    ) => void
    vi.mocked(commands.sendMessage).mockReturnValueOnce(
      new Promise(r => (resolve = r))
    )

    const sending = sendAgentMessage('writer', 'Draft it')
    const pending = messages().at(-1)
    expect(pending).toMatchObject({
      kind: 'user',
      content: 'Draft it',
      pending: true,
    })
    expect(commands.sendMessage).toHaveBeenCalledWith(
      'writer',
      'Draft it',
      [],
      pending?.id
    )

    resolve({
      status: 'ok',
      data: {
        runId: 'r2',
        messageId: pending?.id ?? '',
        conversationTitle: null,
        queued: false,
      },
    })
    await sending
    expect(messages().at(-1)).toMatchObject({ id: pending?.id, pending: false })
  })

  it('removes the optimistic message if Rust refuses it', async () => {
    await loadAgent('writer')
    vi.mocked(commands.sendMessage).mockResolvedValueOnce({
      status: 'error',
      error: 'This conversation is still working on the previous message',
    })

    await sendAgentMessage('writer', 'Too soon')

    expect(messages().map(m => m.id)).toEqual(['u1'])
  })

  it('reloads an agent when agents change (e.g. created from another client)', async () => {
    vi.mocked(commands.getAgentSnapshot).mockClear()

    handleNandiEvent({
      event: 'agent://agents_changed',
      payload: { agentId: 'writer' },
    })

    await vi.waitFor(() =>
      expect(commands.getAgentSnapshot).toHaveBeenCalledWith('writer')
    )
  })

  it('resync reloads every snapshot', async () => {
    vi.mocked(commands.listAgents).mockResolvedValueOnce({
      status: 'ok',
      data: [WRITER],
    })
    vi.mocked(commands.getDashboardSnapshot).mockClear()

    handleNandiEvent({ event: 'nandi://resync', payload: { missed: 3 } })

    await vi.waitFor(() => {
      expect(commands.getDashboardSnapshot).toHaveBeenCalled()
      expect(useAgentStore.getState().agents.writer).toBeDefined()
    })
  })

  it('applies queue_changed as the whole queue state', async () => {
    vi.mocked(commands.getAgentSnapshot).mockResolvedValueOnce({
      status: 'ok',
      data: agentSnapshot(WRITER, 'writer', {
        queue: [queueEntry('q1', 'writer')],
        currentTurn: run('r1', 'writer'),
      }),
    })
    await loadAgent('writer')
    expect(queueIds()).toEqual(['q1'])
    expect(useAgentStore.getState().current.writer?.id).toBe('r1')

    handleNandiEvent({
      event: 'agent://queue_changed',
      payload: {
        agentId: 'writer',
        queue: [
          queueEntry('q2', 'writer'),
          queueEntry('q1', 'writer', { position: 1 }),
        ],
        current: run('r1', 'writer'),
      },
    })
    expect(queueIds()).toEqual(['q2', 'q1'])

    handleNandiEvent({
      event: 'agent://queue_changed',
      payload: { agentId: 'writer', queue: [], current: null },
    })
    expect(queueIds()).toEqual([])
    expect(useAgentStore.getState().current.writer).toBeNull()
  })

  it('lets a queue event that arrives mid-load win over the snapshot', async () => {
    let resolve!: (value: ReturnType<typeof snapshotOk>) => void
    vi.mocked(commands.getAgentSnapshot).mockReturnValueOnce(
      new Promise(r => (resolve = r))
    )
    const loading = loadAgent('writer')

    handleNandiEvent({
      event: 'agent://queue_changed',
      payload: {
        agentId: 'writer',
        queue: [queueEntry('fresh', 'writer')],
        current: run('r1', 'writer'),
      },
    })
    // The snapshot was read before the event
    resolve({
      status: 'ok',
      data: agentSnapshot(WRITER, 'writer', {
        queue: [queueEntry('stale', 'writer')],
      }),
    })
    await loading

    expect(queueIds()).toEqual(['fresh'])
    expect(useAgentStore.getState().current.writer?.id).toBe('r1')
  })

  it('tracks the current turn through run events', async () => {
    await loadAgent('writer')
    handleEvent({
      type: 'run',
      payload: { ...ids, run: run('r1', 'writer') },
    })
    expect(useAgentStore.getState().current.writer?.status).toBe('running')

    handleEvent({
      type: 'run',
      payload: {
        ...ids,
        run: run('r1', 'writer', { status: 'awaitingApproval' }),
      },
    })
    expect(useAgentStore.getState().current.writer?.status).toBe(
      'awaitingApproval'
    )

    // A queued run isn't the current turn
    handleEvent({
      type: 'run',
      payload: { ...ids, run: run('r2', 'writer', { status: 'queued' }) },
    })
    expect(useAgentStore.getState().current.writer?.id).toBe('r1')

    handleEvent({
      type: 'run',
      payload: { ...ids, run: run('r1', 'writer', { status: 'completed' }) },
    })
    expect(useAgentStore.getState().current.writer).toBeNull()
  })

  it('takes a queued message out of the chat until its turn starts', async () => {
    await loadAgent('writer')
    vi.mocked(commands.sendMessage).mockImplementationOnce(
      (_agentId, _text, _attachments, messageId) =>
        Promise.resolve({
          status: 'ok',
          data: {
            runId: 'r2',
            messageId: messageId ?? '',
            conversationTitle: null,
            queued: true,
          },
        })
    )

    await sendAgentMessage('writer', 'Later please')
    expect(messages().map(m => m.id)).toEqual(['u1'])

    // When the run starts, Rust adds the message with the same ID
    const messageId = vi.mocked(commands.sendMessage).mock.calls.at(-1)?.[3]
    handleEvent({
      type: 'message',
      payload: {
        ...ids,
        message: { ...userMessage(messageId ?? '', 'Later please', 'r2') },
      },
    })
    expect(messages().map(m => m.id)).toEqual(['u1', messageId])
  })

  it('refreshes trigger queries on trigger events, without polling', () => {
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
    handleNandiEvent({
      event: 'trigger://fired',
      payload: {
        triggerId: 'orders',
        fireId: 'f1',
        sourceType: 'webhook',
        payload: { order: 1 },
        runId: 'r9',
      },
    })
    const keys = invalidate.mock.calls.map(([filters]) => filters?.queryKey)
    expect(keys).toEqual(
      expect.arrayContaining([
        ['triggers'],
        ['trigger', 'orders'],
        ['triggerFires', 'orders'],
      ])
    )

    invalidate.mockClear()
    handleNandiEvent({ event: 'trigger://changed', payload: {} })
    expect(invalidate.mock.calls.map(([f]) => f?.queryKey)).toEqual(
      expect.arrayContaining([['triggers'], ['trigger']])
    )
    invalidate.mockRestore()
  })

  it('keeps an agent warning until the container is running', async () => {
    await loadAgent('writer')
    handleNandiEvent({
      event: 'agent://warning',
      payload: {
        agentId: 'writer',
        message: 'Tool execution unavailable: Docker is not running',
      },
    })
    expect(useAgentStore.getState().agents.writer?.warning).toBe(
      'Tool execution unavailable: Docker is not running'
    )

    // A reload that still shows the container down keeps the banner
    vi.mocked(commands.getAgentSnapshot).mockResolvedValueOnce({
      status: 'ok',
      data: agentSnapshot(WRITER, 'writer', {
        container: { status: 'error', error: 'Docker is not running' },
      }),
    })
    await loadAgent('writer')
    expect(useAgentStore.getState().agents.writer?.warning).not.toBeNull()

    vi.mocked(commands.getAgentSnapshot).mockResolvedValueOnce({
      status: 'ok',
      data: agentSnapshot(WRITER, 'writer', {
        container: { status: 'running', error: null },
      }),
    })
    await loadAgent('writer')
    expect(useAgentStore.getState().agents.writer?.warning).toBeNull()
  })
})
