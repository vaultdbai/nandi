/**
 * For tests that mock `client`: makes `askStreaming` answer the way the
 * core does (the request ID, then `ask://completed` to subscribers) using
 * the test's `ask` mock, so tests keep asserting on `ask`.
 */
import { vi, type Mock } from 'vitest'

interface MockClient {
  ask: Mock
  subscribe: Mock
  askStreaming?: Mock
  listAskHistory?: Mock
}

export function streamAsksThroughAsk(client: MockClient) {
  const listeners = new Set<(event: unknown) => void>()
  const subscribe = client.subscribe.getMockImplementation()
  client.subscribe.mockImplementation((handler: (event: unknown) => void) => {
    listeners.add(handler)
    const inner = (subscribe?.(handler) as
      { ready: Promise<void>; unsubscribe: () => void } | undefined) ?? {
      ready: Promise.resolve(),
      unsubscribe: () => undefined,
    }
    return {
      ready: inner.ready,
      unsubscribe: () => {
        listeners.delete(handler)
        inner.unsubscribe()
      },
    }
  })
  client.askStreaming = vi.fn(async (request: unknown) => {
    const result = (await client.ask(request)) as {
      status: string
      data?: unknown
      error?: string
    }
    const requestId = crypto.randomUUID()
    setTimeout(() => {
      for (const listener of listeners) {
        listener({
          event: 'ask://completed',
          payload: {
            requestId,
            response: result.status === 'ok' ? result.data : null,
            error: result.status === 'ok' ? null : result.error,
          },
        })
      }
    })
    return { status: 'ok', data: requestId }
  })
  client.listAskHistory ??= vi
    .fn()
    .mockResolvedValue({ status: 'ok', data: [] })
}
