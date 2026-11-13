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

  it('calls the emergency-data routes and returns errors as IPC does', async () => {
    respond(200, null)
    await webClient.saveEmergencyData('draft/1', { a: 1 })
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/emergency-data/draft%2F1')
    expect(init.method).toBe('PUT')
    expect(init.credentials).toBe('same-origin')
    expect(JSON.parse(String(init.body))).toEqual({ a: 1 })

    respond(400, { error: { type: 'FileNotFound' } })
    expect(await webClient.loadEmergencyData('ghost')).toEqual({
      status: 'error',
      error: { type: 'FileNotFound' },
    })

    fetchMock.mockRejectedValueOnce(new TypeError('offline'))
    const offline = await webClient.cleanupOldRecoveryFiles()
    expect(offline.status).toBe('error')
  })

  it('refuses what needs the desktop app', async () => {
    expect((await webClient.pickAttachments('file')).status).toBe('error')
    expect((await webClient.openRunFile('r1', 'a.txt', false)).status).toBe(
      'error'
    )
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
