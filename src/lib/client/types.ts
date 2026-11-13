/**
 * `NandiClient`: everything the UI can ask of the Nandi core.
 *
 * Almost all of it is the generated `commands` (`bindings.ts`): each is
 * declared once in Rust with `rpc::commands!` and runs over Tauri IPC on
 * the desktop or `POST /api/rpc/{name}` in a browser (see `invoke.ts`).
 * Each transport adds only the few methods written by hand: `Transport`,
 * and in the browser, answers to the `HandWrittenCommands`.
 */

import type { commands } from '@/lib/bindings'
import type { NandiEvent } from './events'

/** The generated commands */
type Commands = typeof commands

/**
 * Commands Rust writes by hand (`src-tauri/src/desktop/`), which aren't on
 * `/api/rpc`. The desktop calls them as generated; the web client answers
 * each itself: "use the desktop app", a no-op, or its own route
 * (emergency data, whose errors are typed)
 */
export type HandWrittenCommands = Pick<
  Commands,
  | 'pickAttachments'
  | 'openRunFile'
  | 'toggleCompanionWindow'
  | 'quitNandi'
  | 'placeCompanion'
  | 'setCompanionBubble'
  | 'setCompanionPinned'
  | 'companionFocusRequest'
  | 'saveEmergencyData'
  | 'loadEmergencyData'
  | 'cleanupOldRecoveryFiles'
>

export interface Subscription {
  /**
   * Resolves once events are flowing. Load snapshots only after this, so no
   * event can slip between a snapshot read and the first listener.
   */
  ready: Promise<void>
  unsubscribe: () => void
}

/** What each transport writes by hand, besides the commands */
export interface Transport {
  /** `tauri` (desktop webview) or `web` (browser) */
  readonly transport: 'tauri' | 'web'
  /** Features that need the desktop app */
  readonly supports: {
    /** Native file picker for attachments */
    attachments: boolean
  }
  /** Where webhook/device requests go, e.g. `http://127.0.0.1:17463` */
  getWebOrigin(): Promise<string | null>
  /** Opens `url` in the system browser */
  openExternal(url: string): Promise<void>
  /**
   * Receives every core event. Call once at boot. The web client also
   * emits `nandi://resync` after reconnecting.
   */
  subscribe(handler: (event: NandiEvent) => void): Subscription
}

/** The client: every generated command, and the transport's own methods */
export type NandiClient = Omit<Commands, keyof Transport> & Transport
