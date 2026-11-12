import { commands } from '@/lib/tauri-bindings'
import type { HandWrittenClient, NandiClient } from './types'

/**
 * A transport's client: its hand-written methods, then every generated
 * command. The commands sit on the prototype, so each call looks the
 * command up on `commands` when it runs (tests can swap one).
 */
export function withCommands(handWritten: HandWrittenClient): NandiClient {
  return Object.assign(Object.create(commands) as NandiClient, handWritten)
}
