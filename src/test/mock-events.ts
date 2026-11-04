/**
 * Controllable stand-in for Tauri's `listen()` in tests: `mockListen`
 * registers a handler for a wire name (e.g. `agent://token`), and
 * `emitMockEvent` delivers a payload to it as if Rust had emitted it.
 */

type Handler = (event: { payload: unknown }) => void

const handlers = new Map<string, Set<Handler>>()

export function mockListen(name: string, handler: Handler) {
  const set = handlers.get(name) ?? new Set<Handler>()
  set.add(handler)
  handlers.set(name, set)
  return Promise.resolve(() => {
    set.delete(handler)
  })
}

/** Delivers `payload` to listeners of `name` (e.g. "agent://token"). */
export function emitMockEvent(name: string, payload: unknown) {
  for (const handler of handlers.get(name) ?? []) handler({ payload })
}

export function clearMockEventHandlers() {
  handlers.clear()
}
