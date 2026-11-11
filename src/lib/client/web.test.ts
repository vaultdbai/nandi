import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { webClient } from './web'
import type { NandiEvent } from './events'

/** Minimal controllable WebSocket */
class FakeSocket {
  static instances: FakeSocket[] = []
  onopen: (() => void) | null = null
  onmessage: ((message: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  closed = false

  constructor(public url: string) {
    FakeSocket.instances.push(this)
  }

  open() {
    this.onopen?.()
  }

  receive(frame: unknown) {
    this.onmessage?.({ data: JSON.stringify(frame) })
  }

  drop() {
    this.onclose?.()
  }

  close() {
    this.closed = true
  }
}

const fetchMock = vi.fn()

function respond(status: number, body: unknown) {
  fetchMock.mockResolvedValueOnce({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  })
}

describe('webClient', () => {
  beforeEach(() => {
    FakeSocket.instances = []
    vi.stubGlobal('fetch', fetchMock)
    vi.stubGlobal('WebSocket', FakeSocket)
    fetchMock.mockReset()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('maps calls onto REST routes with JSON bodies', async () => {
    respond(200, { runId: 'r1', messageId: 'm1', conversationTitle: null })

    const result = await webClient.sendMessage('writer', 'hi', ['a1'], 'm1')

    expect(result).toEqual({
      status: 'ok',
      data: { runId: 'r1', messageId: 'm1', conversationTitle: null },
    })
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/agents/writer/messages')
    expect(init.method).toBe('POST')
    expect(init.credentials).toBe('same-origin')
    expect(JSON.parse(String(init.body))).toEqual({
      text: 'hi',
      attachments: ['a1'],
      clientMessageId: 'm1',
    })
  })

  it('scopes approvals by agent and encodes path segments', async () => {
    respond(200, null)
    await webClient.approveAction('@studio', 'ap/1', 'approved')
    expect(fetchMock.mock.calls[0]?.[0]).toBe('/api/agents/%40studio/approve')
  })

  it('answers a hand-over by its approval', async () => {
    respond(200, null)
    await webClient.answerHandOver('ap/1', {
      outcome: 'changed',
      finalText: 'Final words',
    })
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/approvals/ap%2F1/hand-over')
    expect(JSON.parse(String(init.body))).toEqual({
      outcome: 'changed',
      finalText: 'Final words',
    })
  })

  it('returns errors in the same shape as IPC', async () => {
    respond(400, { error: 'Agent not found' })
    expect(await webClient.getAgentSnapshot('ghost')).toEqual({
      status: 'error',
      error: 'Agent not found',
    })

    fetchMock.mockRejectedValueOnce(new TypeError('offline'))
    const offline = await webClient.listAgents()
    expect(offline.status).toBe('error')
  })

  it('unwraps container ids and refuses native file picking', async () => {
    respond(200, { containerId: 'c1' })
    expect(await webClient.startAgent('writer')).toEqual({
      status: 'ok',
      data: 'c1',
    })
    expect((await webClient.pickAttachments('file')).status).toBe('error')
    expect(webClient.supports.attachments).toBe(false)
  })

  it('streams WebSocket events and resyncs after reconnecting', async () => {
    vi.useFakeTimers()
    const received: NandiEvent[] = []
    const subscription = webClient.subscribe(event => received.push(event))

    const first = FakeSocket.instances[0]
    expect(first?.url).toMatch(/\/ws$/)
    first?.open()
    await subscription.ready

    first?.receive({ event: 'agent://run', payload: { agentId: 'a' } })
    expect(received).toEqual([
      { event: 'agent://run', payload: { agentId: 'a' } },
    ])

    // Connection lost: reconnect with backoff, then ask for a resync
    first?.drop()
    await vi.advanceTimersByTimeAsync(1_000)
    const second = FakeSocket.instances[1]
    second?.open()
    expect(received.at(-1)).toEqual({ event: 'nandi://resync', payload: {} })

    subscription.unsubscribe()
    expect(second?.closed).toBe(true)
  })

  it('shares one WebSocket between subscribers', () => {
    const first = webClient.subscribe(vi.fn())
    const second = webClient.subscribe(vi.fn())

    expect(FakeSocket.instances).toHaveLength(1)

    first.unsubscribe()
    expect(FakeSocket.instances[0]?.closed).toBe(false)
    second.unsubscribe()
    expect(FakeSocket.instances[0]?.closed).toBe(true)
  })
})
