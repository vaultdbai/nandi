/**
 * `NandiClient` over HTTP + WebSocket, for browsers. The page is served by
 * the Nandi web server, so requests are same-origin and authenticated by its
 * HttpOnly cookie (set when the page was opened with `?token=`).
 */

import type { Result } from '@/lib/bindings'
import { commands } from '@/lib/tauri-bindings'
import type { NandiEvent } from './events'
import { createEventBus } from './event-bus'
import type { NandiClient } from './types'
import { withCommands } from './with-commands'

const NOT_SUPPORTED = 'Not supported in the browser. Use the Nandi desktop app.'
const RECONNECT_MIN_MS = 1_000
const RECONNECT_MAX_MS = 30_000
/** Boot proceeds after this even if the socket can't connect yet; the
 * reconnect then triggers a resync */
const READY_TIMEOUT_MS = 5_000

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE'

/** Calls the REST API; errors become `{ status: 'error' }` like IPC. */
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

/** `?a=1&b=x` from the set values (`null`/`undefined` are left out) */
function query(
  params: Record<string, string | number | null | undefined>
): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value !== null && value !== undefined) search.set(key, String(value))
  }
  const encoded = search.toString()
  return encoded ? `?${encoded}` : ''
}

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

export const webClient: NandiClient = withCommands({
  transport: 'web',
  supports: { attachments: false },

  // The native picker would open on the host, not in this browser
  pickAttachments: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),

  // It would open on the host, not in this browser
  openRunFile: () => Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),

  saveEmergencyData: (filename, data) =>
    request('PUT', `/emergency-data/${id(filename)}`, data),
  loadEmergencyData: filename =>
    request('GET', `/emergency-data/${id(filename)}`),
  cleanupOldRecoveryFiles: () => request('POST', '/emergency-data/cleanup'),

  // The page is served by the Nandi web server itself
  getWebOrigin: () => Promise.resolve(location.origin),

  // Agents repo, pilots, run history, audit, connectors, and the network
  saveAgent: (agentId, input) =>
    request('POST', `/repo/agents/${id(agentId)}/save`, input),
  saveAgentInstructions: (agentId, instructions) =>
    request('POST', `/repo/agents/${id(agentId)}/instructions`, {
      instructions,
    }),
  resetAgentInstructions: agentId =>
    request('DELETE', `/repo/agents/${id(agentId)}/instructions`),
  agentPlaybooks: agentId =>
    request('GET', `/repo/agents/${id(agentId)}/playbooks`),
  saveAgentPlaybook: (agentId, input, previous) =>
    request('POST', `/repo/agents/${id(agentId)}/playbooks`, {
      input,
      previous,
    }),
  deleteAgentPlaybook: (agentId, name) =>
    request('DELETE', `/repo/agents/${id(agentId)}/playbooks/${id(name)}`),
  loadEntity: (kind, entityId) =>
    request('GET', `/repo/entities/${id(kind)}/${id(entityId)}`),
  gitStatus: () => request('GET', '/repo/git/status'),
  gitDiff: path => request('GET', `/repo/git/diff${query({ path })}`),
  gitCommit: message => request('POST', '/repo/git/commit', { message }),

  // The route still names the field `githubOrg`; it carries the registry URL

  toggleCompanionWindow: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  // The server keeps running; a browser tab has nothing to quit
  quitNandi: () => Promise.resolve(),
  // No companion window in a browser
  placeCompanion: () => Promise.resolve({ status: 'ok', data: null }),
  setCompanionBubble: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  // Saves the preference; there is no window to keep on top
  setCompanionPinned: pinned => commands.saveCompanionPinned(pinned),
  // The web build's companion lives in the page (`watchInCompanion`)
  companionFocusRequest: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),

  // The relay connection lives in the core; no browser route for this yet

  openExternal: url => {
    window.open(url, '_blank', 'noopener,noreferrer')
    return Promise.resolve()
  },

  subscribe: handler => socketEvents.subscribe(handler),
})
