/**
 * Domain types for the Agent Console.
 *
 * Agents, chat messages, runs, and approvals come from Rust (tauri-specta
 * generated types).
 */

export type {
  AgentInfo,
  AgentStatus,
  AttachmentInfo,
  ChatMessage,
} from '@/lib/tauri-bindings'
export type { UiMessage } from '@/lib/store-reducers'
