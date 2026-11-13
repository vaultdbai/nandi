import { commands } from '@/lib/tauri-bindings'
import type { HandWrittenCommands, NandiClient, Transport } from './types'

/**
 * A transport's client: its own methods, then every generated command. The
 * commands sit on the prototype, so each call looks the command up on
 * `commands` when it runs (tests can swap one).
 */
export function withCommands(
  own: Transport & Partial<HandWrittenCommands>
): NandiClient {
  return Object.assign(Object.create(commands) as NandiClient, own)
}
