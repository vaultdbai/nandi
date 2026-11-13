/** `NandiClient` over Tauri IPC: the generated tauri-specta bindings. */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { commands } from '@/lib/tauri-bindings'
import { CORE_EVENT_NAMES, type NandiEvent } from './events'
import { createEventBus } from './event-bus'
import type { NandiClient } from './types'
import { withCommands } from './with-commands'

/** One `listen()` per core event, shared by every subscriber */
const coreEvents = createEventBus(emit => {
  const subscriptions: Promise<UnlistenFn>[] = CORE_EVENT_NAMES.map(name =>
    listen(name, e => emit({ event: name, payload: e.payload } as NandiEvent))
  )
  return {
    ready: Promise.all(subscriptions).then(() => undefined),
    unsubscribe: () => {
      for (const subscription of subscriptions) {
        void subscription.then(unlisten => unlisten())
      }
    },
  }
})

/** The desktop calls every command as generated, its own ones included */
export const tauriClient: NandiClient = withCommands({
  transport: 'tauri',
  supports: { attachments: true },

  getWebOrigin: async () => {
    const result = await commands.getWebOrigin()
    return result.status === 'ok' ? result.data : null
  },
  openExternal: url => openUrl(url),
  subscribe: handler => coreEvents.subscribe(handler),
})
