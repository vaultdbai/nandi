/**
 * The Nandi client for the current runtime: Tauri IPC inside the desktop
 * app, HTTP + WebSocket in a browser. UI code imports `client` and never
 * calls `invoke()`/`listen()` directly.
 */

import { isTauri } from '@tauri-apps/api/core'
import { tauriClient } from './tauri'
import type { NandiClient } from './types'
import { webClient } from './web'

export const client: NandiClient = isTauri() ? tauriClient : webClient

/** Whether desktop-only APIs (windows, menus, OS plugins) are available */
export const isDesktop = (): boolean => client.transport === 'tauri'

export type { NandiClient, Subscription } from './types'
export type { NandiEvent, NandiEventName } from './events'
