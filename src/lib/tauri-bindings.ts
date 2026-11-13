/**
 * Re-export generated Tauri bindings with project conventions
 *
 * This file provides type-safe access to all Tauri commands.
 * Types are auto-generated from Rust by tauri-specta.
 *
 * @example
 * ```typescript
 * import { commands, unwrapResult } from '@/lib/tauri-bindings'
 *
 * // In TanStack Query - let errors propagate
 * const prefs = unwrapResult(await commands.loadPreferences())
 *
 * // In event handlers - explicit error handling
 * const result = await commands.savePreferences(prefs)
 * if (result.status === 'error') {
 *   toast.error(result.error)
 * }
 * ```
 *
 * @see docs/developer/tauri-commands.md for full documentation
 */

export { commands, type Result } from './bindings'
export type {
  ActivityAction,
  ActivityItem,
  AgentApprovalRequest,
  AgentApprovalResolved,
  AgentInfo,
  AgentMessageAdded,
  AgentQueueChanged,
  AgentRunUpdated,
  AgentSnapshot,
  AgentStatus,
  AgentToken,
  AgentToolCall,
  AgentToolResult,
  AgentTurnDone,
  AgentWarning,
  AppPreferences,
  ApprovalDecision,
  ApprovalStatus,
  AttachmentInfo,
  AttachmentKind,
  AttentionItem,
  AwsCapabilities,
  AwsCredentialStatus,
  AwsMode,
  AwsServiceGrant,
  Capabilities,
  ChatMessage,
  ContainerState,
  ContainerStatus,
  ConversationSnapshot,
  DashboardActivity,
  DashboardAttention,
  DashboardSnapshot,
  EgressKind,
  EgressLogEntry,
  EgressRoute,
  EgressStatus,
  HostRule,
  JsonValue,
  KeyedProvider,
  LlmKeyStatus,
  NetworkCapabilities,
  PendingApproval,
  QueueEntry,
  RecoveryError,
  RunDetail,
  RunEventRecord,
  RunStatus,
  RunSummary,
  SavedTrigger,
  SendMessageResult,
  SourceType,
  StudioConversationSummary,
  Trigger,
  TriggerError,
  TriggerFire,
  TriggerFired,
  TriggerFireStatus,
  FireRunStatus,
  TriggerInput,
  TriggerRateLimited,
  TriggersChanged,
  TriggerSourceType,
  TriggerTargetType,
  Workflow,
  WorkflowRun,
  WorkflowStepRun,
  WorkflowStepStatus,
  WorkflowSummary,
  // Wave 1 interfaces
  AgentDefinitionCapabilities,
  AgentDefinitionInput,
  AuditEntry,
  ConnectorChanged,
  ConnectorInfo,
  BackfillAction,
  BackfillEstimate,
  BackfillPhase,
  BackfillRange,
  BackfillStatus,
  EmailAccountState,
  EmailChanged,
  EmailProviderStatus,
  EmailSettingsInput,
  EmailSignInPoll,
  EmailStatus,
  MailProvider,
  SignInPhase,
  SignInProblem,
  SignInProblemKind,
  ConnectorsChanged,
  ConnectorStatus,
  DateRange,
  DeviceFlowState,
  GitFile,
  GitStatus,
  PilotChanged,
  PilotConsentNeeded,
  PilotDefinition,
  PilotStatus,
  RunFilters,
  RunPage,
} from './bindings'

/**
 * Helper to unwrap a Result type, throwing on error. A string error (every
 * command's) becomes an `Error`, so `error.message` works wherever it is
 * read; any other error is thrown as it is.
 */
export function unwrapResult<T, E>(
  result: { status: 'ok'; data: T } | { status: 'error'; error: E }
): T {
  if (result.status === 'ok') {
    return result.data
  }
  throw typeof result.error === 'string'
    ? new Error(result.error)
    : result.error
}
