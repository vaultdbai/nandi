/**
 * The `invoke` the generated `commands` (`bindings.ts`) call: Tauri IPC in
 * the desktop app, `POST /api/rpc/{name}` in a browser. Either way a
 * command's error is thrown as its value (not an `Error`), which the
 * generated wrapper turns into `{ status: 'error', error }`.
 */

import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core'

/** Calls a command over HTTP; its JSON body is the command's arguments */
export async function rpcInvoke<T>(
  name: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  let response: Response
  try {
    response = await fetch(`/api/rpc/${encodeURIComponent(name)}`, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(args),
    })
  } catch (error) {
    throw `Can't reach Nandi: ${String(error)}`
  }
  const json: unknown = await response.json().catch(() => null)
  if (response.ok) return json as T
  throw (
    (json as { error?: unknown } | null)?.error ??
    `Request failed (${response.status})`
  )
}

export const invoke = isTauri() ? tauriInvoke : rpcInvoke
