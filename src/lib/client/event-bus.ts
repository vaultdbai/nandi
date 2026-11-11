/**
 * One transport subscription shared by every `client.subscribe` caller.
 * The first subscriber connects the transport (Tauri listeners, or the
 * WebSocket), later ones only join the fan-out in JS, and the last one to
 * leave disconnects it. Dozens of hooks subscribe at once, so without this
 * each would register its own listener set or open its own socket.
 */

import { logger } from '@/lib/logger'
import type { NandiEvent } from './events'
import type { Subscription } from './types'

type Handler = (event: NandiEvent) => void

/** Connects the transport once; `emit` delivers each event it receives */
type Connect = (emit: Handler) => Subscription

export interface EventBus {
  subscribe(handler: Handler): Subscription
}

export function createEventBus(connect: Connect): EventBus {
  const handlers = new Set<Handler>()
  let source: Subscription | null = null

  const emit: Handler = event => {
    // A copy, so a handler that unsubscribes mid-delivery doesn't skip another
    for (const handler of [...handlers]) {
      try {
        handler(event)
      } catch (error) {
        // One failing handler must not starve the others
        logger.error('Event handler failed', { event: event.event, error })
      }
    }
  }

  return {
    subscribe(handler) {
      // A function can subscribe twice; each subscription is its own entry
      const entry: Handler = event => handler(event)
      handlers.add(entry)
      source ??= connect(emit)
      const { ready } = source

      return {
        ready,
        unsubscribe: () => {
          if (!handlers.delete(entry) || handlers.size > 0) return
          source?.unsubscribe()
          source = null
        },
      }
    },
  }
}
