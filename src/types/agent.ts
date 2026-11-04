/**
 * Domain types for the Agent Console.
 *
 * Agents, chat messages, runs, and approvals come from Rust (tauri-specta
 * generated types). Permissions, connectors, and git state are still
 * frontend-only mock data.
 */

export type {
  AgentInfo,
  AgentStatus,
  ApprovalStatus,
  AttachmentInfo,
  ChatMessage,
  RunStatus,
} from '@/lib/tauri-bindings'
export type { UiMessage } from '@/lib/store-reducers'

export type PermissionLevel = 'allowed' | 'ask' | 'denied'

export interface AgentPermission {
  id: string
  scope: string
  detail: string
  level: PermissionLevel
}

export interface AgentConnector {
  id: string
  name: string
  description: string
  connected: boolean
}

export interface GitCommit {
  sha: string
  message: string
  author: string
  /** ISO 8601 timestamp */
  createdAt: string
}

export interface AgentGitState {
  repository: string
  branch: string
  changedFiles: number
  recentCommits: GitCommit[]
}
