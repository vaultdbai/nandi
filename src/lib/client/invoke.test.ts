import { describe, it, expect, afterEach, vi } from 'vitest'
import { rpcInvoke } from './invoke'

const fetchMock = vi.fn()

function respond(status: number, body: unknown) {
  fetchMock.mockResolvedValueOnce({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  })
}

describe('rpcInvoke', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    fetchMock.mockReset()
  })

  it('posts the arguments to /api/rpc/{name} and returns the result', async () => {
    vi.stubGlobal('fetch', fetchMock)
    respond(200, 'r1')

    await expect(rpcInvoke('request_for_run', { runId: 'x' })).resolves.toBe(
      'r1'
    )
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/rpc/request_for_run')
    expect(init.method).toBe('POST')
    expect(JSON.parse(String(init.body))).toEqual({ runId: 'x' })
  })

  it('sends an empty object for a command without arguments', async () => {
    vi.stubGlobal('fetch', fetchMock)
    respond(200, null)

    await rpcInvoke('get_chief_of_staff_digest')
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(init.body).toBe('{}')
  })

  it('throws the error text, as Tauri IPC does', async () => {
    vi.stubGlobal('fetch', fetchMock)
    respond(400, { error: 'No such request' })
    await expect(rpcInvoke('get_request', { id: 'x' })).rejects.toBe(
      'No such request'
    )

    fetchMock.mockRejectedValueOnce(new TypeError('offline'))
    await expect(rpcInvoke('get_request', { id: 'x' })).rejects.toMatch(
      /^Can't reach Nandi/
    )
  })
})
