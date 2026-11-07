/**
 * Every event the core emits, by wire name. Both transports deliver the
 * same `{ event, payload }` shape (Tauri listeners, or WebSocket frames).
 */

import type {
  AgentApprovalRequest,
  AgentApprovalResolved,
  AgentMessageAdded,
  AgentQueueChanged,
  AgentRunCompacted,
  AgentRunUpdated,
  AgentsChanged,
  AgentToken,
  AgentToolCall,
  AgentToolResult,
  AgentToolsLoaded,
  AgentTurnAborted,
  AgentTurnDone,
  AgentWarning,
  ConnectorChanged,
  ConnectorsChanged,
  EmailChanged,
  DashboardActivity,
  DashboardAttention,
  DelegationTaskCompleted,
  DelegationTaskFailed,
  DelegationTaskStarted,
  RequestReturned,
  MarketplaceError,
  MarketplaceForked,
  MarketplaceInstalled,
  MarketplaceProgress,
  MarketplaceUninstalled,
  MarketplaceUpdateApplied,
  MarketplaceUpdateAvailable,
  PilotChanged,
  PilotConsentNeeded,
  RelayAgentOffline,
  RelayAgentOnline,
  RelayStatus,
  RelaySealRefused,
  MemberKeyChanged,
  RepoFileChanged,
  RepoValidationFailed,
  TeamChanged,
  TriggerError,
  TriggerFired,
  TriggerRateLimited,
  TriggersChanged,
  WorkflowDone,
  WorkflowFailed,
  WorkflowsChanged,
  WorkflowStepAwaitingApproval,
  WorkflowStepDone,
  WorkflowStepStarted,
  AskCompleted,
  AskStreamingDelta,
  StudioArtifactLoaded,
  StudioArtifactSaved,
  StudioEditPreviewed,
  WorkItemAdded,
  WorkItemRemoved,
  WorkItemUpdated,
  RequestUpdated,
  TaskReassigned,
  PendingManualStep,
  ManualStepCompleted,
  RepoLlmConfigStatus,
} from '@/lib/bindings'

interface NandiEventMap {
  'agent://run': AgentRunUpdated
  'agent://message': AgentMessageAdded
  'agent://token': AgentToken
  'agent://tool_call': AgentToolCall
  'agent://tool_result': AgentToolResult
  'agent://approval_request': AgentApprovalRequest
  'agent://approval_resolved': AgentApprovalResolved
  'agent://turn_done': AgentTurnDone
  'agent://agents_changed': AgentsChanged
  'agent://warning': AgentWarning
  'agent://queue_changed': AgentQueueChanged
  'dashboard://attention': DashboardAttention
  'dashboard://activity': DashboardActivity
  'workflow://step_started': WorkflowStepStarted
  'workflow://step_awaiting_approval': WorkflowStepAwaitingApproval
  'workflow://step_done': WorkflowStepDone
  'workflow://done': WorkflowDone
  'workflow://failed': WorkflowFailed
  'workflow://changed': WorkflowsChanged
  'trigger://fired': TriggerFired
  'trigger://rate_limited': TriggerRateLimited
  'trigger://error': TriggerError
  'trigger://changed': TriggersChanged
  'marketplace://installed': MarketplaceInstalled
  'marketplace://uninstalled': MarketplaceUninstalled
  'marketplace://forked': MarketplaceForked
  'marketplace://update_available': MarketplaceUpdateAvailable
  'marketplace://update_applied': MarketplaceUpdateApplied
  'marketplace://error': MarketplaceError
  'marketplace://progress': MarketplaceProgress
  'repo://file_changed': RepoFileChanged
  'repo://validation_failed': RepoValidationFailed
  'pilot://changed': PilotChanged
  /** One line of a pilot's install output, while it installs */
  'pilot://install_output': {
    pilotId: string
    stream: 'stdout' | 'stderr'
    line: string
  }
  /** A run left out a declared pilot the user hasn't granted yet */
  'pilot://consent_needed': PilotConsentNeeded
  'connector://changed': ConnectorChanged
  'connectors://changed': ConnectorsChanged
  'email://changed': EmailChanged
  /** A connector grant was given or removed */
  'grants://changed': unknown
  /** A calendar or drive connected, disconnected or was checked */
  'workspace://changed': unknown
  /** A sandbox image build's output line, or its end (`finished`) */
  'images://build': {
    image: 'harness' | 'runtime'
    line: string | null
    finished: boolean | null
    error: string | null
  }
  // Wave 2A
  'relay://status': RelayStatus
  'relay://agent_online': RelayAgentOnline
  'relay://agent_offline': RelayAgentOffline
  /** Something another person sent didn't open; nothing in it was used */
  'relay://seal_refused': RelaySealRefused
  /** The relay shows other keys for a member than the pinned ones */
  'network://member_key_changed': MemberKeyChanged
  'team://changed': TeamChanged
  'delegation://task_started': DelegationTaskStarted
  'delegation://task_completed': DelegationTaskCompleted
  /** Failed, cancelled, or timed out */
  'delegation://task_failed': DelegationTaskFailed
  /** A request you sent came back while nobody watched (an Inbox item) */
  'request://returned': RequestReturned
  // Wave final
  /** A middleware stopped the run (turn limit, cost cap, ...) */
  'agent://turn_aborted': AgentTurnAborted
  /** The run ended as `compacted`; `newRunId` continues it */
  'agent://run_compacted': AgentRunCompacted
  /** `tool_search` added tools to the run */
  'agent://tools_loaded': AgentToolsLoaded
  // Local-identity freeze. Payloads are typed by the tracks that emit them
  'remote://status_changed': unknown
  'remote://push_completed': unknown
  'remote://pull_completed': unknown
  'remote://error': unknown
  'task://state_changed': unknown
  'task://progress': unknown
  'task://clarification_requested': unknown
  'task://clarification_answered': unknown
  'task://timed_out': unknown
  'task://circuit_opened': unknown
  'gateway://config_changed': unknown
  'gateway://caller_updated': unknown
  'gateway://rate_limited': unknown
  /** A request held for the owner's approval opened or was decided */
  'gateway://hold_changed': unknown
  /** Work assigned to the owner opened or closed */
  'gateway://assignment_changed': unknown
  /** A note was saved or deleted (My notes) */
  'notes://changed': unknown
  'network://member_online': unknown
  'network://member_offline': unknown
  'network://membership_changed': unknown
  'identity://changed': unknown
  'team://roster_changed': unknown
  'member://status_changed': unknown
  'trust://relationship_updated': unknown
  /** Someone was refused for trust, or their request was answered (task 12) */
  'trust://request_received': { instanceId: string }
  // Wave A. Payloads are
  // typed by the tracks that emit them
  'capability://created': unknown
  'capability://updated': unknown
  'capability://deleted': unknown
  'capability://ownership_transferred': unknown
  'provider://registered': unknown
  'provider://unregistered': unknown
  'registry://synced': unknown
  'registry://sync_failed': unknown
  'tool://catalog_changed': unknown
  'tool://usage_recorded': unknown
  'evidence://aggregated': unknown
  // Wave B. Payloads are
  // typed by the tracks that emit them
  'harness://detected': unknown
  'harness://health_changed': unknown
  'harness://session_started': unknown
  'harness://session_ended': unknown
  'harness://turn_delta': unknown
  'harness://turn_finished': unknown
  'harness://permission_request': unknown
  'harness://error': unknown
  'aws://profile_changed': unknown
  'aws://credentials_expired': unknown
  'bedrock://models_refreshed': unknown
  'bedrock://gateway_status': unknown
  // Wave CD (see docs/developer/interaction-layer.md)
  'ask://streaming_delta': AskStreamingDelta
  'ask://completed': AskCompleted
  'studio://artifact_loaded': StudioArtifactLoaded
  'studio://artifact_saved': StudioArtifactSaved
  'studio://edit_previewed': StudioEditPreviewed
  'work://item_updated': WorkItemUpdated
  'work://item_added': WorkItemAdded
  'work://item_removed': WorkItemRemoved
  /** A local Request or one of its Steps changed (task 14) */
  'request://updated': RequestUpdated
  // Wave EF (see docs/developer/network-and-reassignment.md)
  /** Someone new appeared in the network directory */
  'network://member_joined': unknown
  /** A member went offline */
  'network://member_left': unknown
  /** A member's gateway card changed */
  'network://profile_updated': unknown
  'task://reassigned': TaskReassigned
  'workflow://manual_step_ready': PendingManualStep
  'workflow://manual_step_completed': ManualStepCompleted
  'config://repo_llm_changed': RepoLlmConfigStatus
  /**
   * The client may have missed events (WebSocket reconnect or lag):
   * reload snapshots. Never sent over Tauri IPC.
   */
  'nandi://resync': { missed?: number }
}

export type NandiEventName = keyof NandiEventMap

export type NandiEvent = {
  [K in NandiEventName]: { event: K; payload: NandiEventMap[K] }
}[NandiEventName]

/** Events the core emits (everything except the client-side resync) */
export const CORE_EVENT_NAMES = [
  'agent://run',
  'agent://message',
  'agent://token',
  'agent://tool_call',
  'agent://tool_result',
  'agent://approval_request',
  'agent://approval_resolved',
  'agent://turn_done',
  'agent://agents_changed',
  'agent://warning',
  'agent://queue_changed',
  'dashboard://attention',
  'dashboard://activity',
  'workflow://step_started',
  'workflow://step_awaiting_approval',
  'workflow://step_done',
  'workflow://done',
  'workflow://failed',
  'workflow://changed',
  'trigger://fired',
  'trigger://rate_limited',
  'trigger://error',
  'trigger://changed',
  'marketplace://installed',
  'marketplace://uninstalled',
  'marketplace://forked',
  'marketplace://update_available',
  'marketplace://update_applied',
  'marketplace://error',
  'marketplace://progress',
  'repo://file_changed',
  'repo://validation_failed',
  'pilot://changed',
  'pilot://install_output',
  'pilot://consent_needed',
  'connector://changed',
  'connectors://changed',
  'grants://changed',
  'workspace://changed',
  'images://build',
  'relay://status',
  'relay://agent_online',
  'relay://agent_offline',
  'relay://seal_refused',
  'network://member_key_changed',
  'team://changed',
  'delegation://task_started',
  'delegation://task_completed',
  'delegation://task_failed',
  'request://returned',
  'agent://turn_aborted',
  'agent://run_compacted',
  'agent://tools_loaded',
  'remote://status_changed',
  'remote://push_completed',
  'remote://pull_completed',
  'remote://error',
  'task://state_changed',
  'task://progress',
  'task://clarification_requested',
  'task://clarification_answered',
  'task://timed_out',
  'task://circuit_opened',
  'gateway://config_changed',
  'gateway://caller_updated',
  'gateway://rate_limited',
  'gateway://hold_changed',
  'gateway://assignment_changed',
  'notes://changed',
  'network://member_online',
  'network://member_offline',
  'network://membership_changed',
  'identity://changed',
  'team://roster_changed',
  'member://status_changed',
  'trust://relationship_updated',
  'trust://request_received',
  'capability://created',
  'capability://updated',
  'capability://deleted',
  'capability://ownership_transferred',
  'provider://registered',
  'provider://unregistered',
  'registry://synced',
  'registry://sync_failed',
  'tool://catalog_changed',
  'tool://usage_recorded',
  'evidence://aggregated',
  'harness://detected',
  'harness://health_changed',
  'harness://session_started',
  'harness://session_ended',
  'harness://turn_delta',
  'harness://turn_finished',
  'harness://permission_request',
  'harness://error',
  'aws://profile_changed',
  'aws://credentials_expired',
  'bedrock://models_refreshed',
  'bedrock://gateway_status',
  'ask://streaming_delta',
  'ask://completed',
  'studio://artifact_loaded',
  'studio://artifact_saved',
  'studio://edit_previewed',
  'work://item_updated',
  'work://item_added',
  'work://item_removed',
  'request://updated',
  'network://member_joined',
  'network://member_left',
  'network://profile_updated',
  'task://reassigned',
  'workflow://manual_step_ready',
  'workflow://manual_step_completed',
  'config://repo_llm_changed',
] as const satisfies readonly NandiEventName[]
