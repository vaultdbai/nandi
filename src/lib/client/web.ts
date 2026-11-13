/**
 * `NandiClient` over HTTP + WebSocket, for browsers. The page is served by
 * the Nandi web server, so requests are same-origin and authenticated by its
 * HttpOnly cookie (set when the page was opened with `?token=`).
 */

import type { Result } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import type { NandiEvent } from './events'
import { createEventBus } from './event-bus'
import type { HandWrittenCommands, NandiClient } from './types'
import { withCommands } from './with-commands'

const NOT_SUPPORTED = 'Not supported in the browser. Use the Nandi desktop app.'
const RECONNECT_MIN_MS = 1_000
const RECONNECT_MAX_MS = 30_000
/** Boot proceeds after this even if the socket can't connect yet; the
 * reconnect then triggers a resync */
const READY_TIMEOUT_MS = 5_000

type Method = 'GET' | 'POST' | 'PUT'

/**
 * Calls a hand-written route (only emergency data has one; every other
 * command goes through `invoke.ts`). Errors become `{ status: 'error' }`
 * like IPC.
 */
async function request<T, E = string>(
  method: Method,
  path: string,
  body?: unknown
): Promise<Result<T, E>> {
  try {
    const response = await fetch(`/api${path}`, {
      method,
      credentials: 'same-origin',
      headers:
        body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    const json: unknown = await response.json().catch(() => null)
    if (response.ok) return { status: 'ok', data: json as T }
    const error =
      (json as { error?: E } | null)?.error ??
      (`Request failed (${response.status})` as E)
    return { status: 'error', error }
  } catch (error) {
    return {
      status: 'error',
      error: `Can't reach Nandi: ${String(error)}` as E,
    }
  }
}

const id = encodeURIComponent

const notSupported = () =>
  Promise.resolve({ status: 'error' as const, error: NOT_SUPPORTED })

/**
 * One WebSocket for the page, shared by every subscriber. It reconnects
 * with backoff and asks for a resync, as events sent meanwhile are lost
 */
const socketEvents = createEventBus(emit => {
  let socket: WebSocket | null = null
  let retryDelay = RECONNECT_MIN_MS
  let retryTimer: ReturnType<typeof setTimeout> | undefined
  let connectedBefore = false
  let closed = false
  let markReady: () => void = () => undefined
  const ready = new Promise<void>(resolve => {
    markReady = resolve
    setTimeout(resolve, READY_TIMEOUT_MS)
  })

  const connect = () => {
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
    socket = new WebSocket(`${protocol}//${location.host}/ws`)

    socket.onopen = () => {
      retryDelay = RECONNECT_MIN_MS
      // Anything emitted while disconnected is only in the DB now
      if (connectedBefore) emit({ event: 'nandi://resync', payload: {} })
      connectedBefore = true
      markReady()
    }
    socket.onmessage = message => {
      try {
        emit(JSON.parse(String(message.data)) as NandiEvent)
      } catch {
        // Ignore malformed frames
      }
    }
    socket.onclose = () => {
      if (closed) return
      retryTimer = setTimeout(connect, retryDelay)
      retryDelay = Math.min(retryDelay * 2, RECONNECT_MAX_MS)
    }
  }

  connect()
  return {
    ready,
    unsubscribe: () => {
      closed = true
      clearTimeout(retryTimer)
      socket?.close()
    },
  }
})

/** The hand-written commands, answered without the desktop app */
const handWritten: HandWrittenCommands = {
  // The native picker would open on the host, not in this browser
  pickAttachments: () => notSupported(),
  // It would open on the host, not in this browser
  openRunFile: () => notSupported(),

  // Typed errors, so these have their own routes (`http/recovery.rs`)
  saveEmergencyData: (filename, data) =>
    request('PUT', `/emergency-data/${id(filename)}`, data),
  loadEmergencyData: filename =>
    request('GET', `/emergency-data/${id(filename)}`),
  cleanupOldRecoveryFiles: () => request('POST', '/emergency-data/cleanup'),

  // There is no companion window in a browser
  toggleCompanionWindow: () => notSupported(),
  // The server keeps running; a browser tab has nothing to quit
  quitNandi: () => Promise.resolve(),
  placeCompanion: () => Promise.resolve({ status: 'ok', data: null }),
  setCompanionBubble: () => notSupported(),
  // Saves the preference; there is no window to keep on top
  setCompanionPinned: pinned => commands.saveCompanionPinned(pinned),
  // The web build's companion lives in the page (`watchInCompanion`)
  companionFocusRequest: () => notSupported(),
}

export const webClient: NandiClient = withCommands({
  transport: 'web',
  supports: { attachments: false },
  ...handWritten,

  // The page is served by the Nandi web server itself
  getWebOrigin: () => Promise.resolve(location.origin),
  openExternal: url => {
    window.open(url, '_blank', 'noopener,noreferrer')
    return Promise.resolve()
  },
  subscribe: handler => socketEvents.subscribe(handler),
})
