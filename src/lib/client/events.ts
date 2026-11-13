/**
 * Every event the core emits, by wire name. Both transports deliver the
 * same `{ event, payload }` shape (Tauri listeners, or WebSocket frames).
 */

import type {
  GroupsChanged,
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
  LiveChange,
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
  PullProgress,
  SandboxImage,
  ToolBundle,
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
  RunPlanUpdated,
  KnowMeChanged,
  RepoRebuildStarted,
  RepoRebuildCompleted,
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
  'repo://rebuild_started': RepoRebuildStarted
  'repo://rebuild_completed': RepoRebuildCompleted
  /** A file the app shows changed on disk, and who changed it */
  'live://changed': LiveChange
  /** A run's plan was set or one of its steps changed */
  'run://plan_updated': RunPlanUpdated
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
  /** The know-me to-do or the answers changed */
  'knowme://changed': KnowMeChanged
  /** A connector grant was given or removed */
  'grants://changed': unknown
  /** A calendar or drive connected, disconnected or was checked */
  'workspace://changed': unknown
  /**
   * Getting a sandbox image: a line of output, the download's progress, or
   * its end (`finished`). The advanced local build reports here too
   */
  'images://pull': {
    image: SandboxImage
    /** The advanced local build's output, not a download's */
    build: boolean
    line: string | null
    progress: PullProgress | null
    finished: boolean | null
    error: string | null
  }
  /** A first-use tool bundle's install (Claude Code): a line, or its end */
  'images://tools': {
    bundle: ToolBundle
    /** About how much it downloads */
    downloadBytes: number
    line: string | null
    finished: boolean | null
    error: string | null
  }
  'relay://status': RelayStatus
  'relay://agent_online': RelayAgentOnline
  'relay://agent_offline': RelayAgentOffline
  /** Something another person sent didn't open; nothing in it was used */
  'relay://seal_refused': RelaySealRefused
  /** The relay shows other keys for a member than the pinned ones */
  'network://member_key_changed': MemberKeyChanged
  /** A group I'm in (or was invited to) changed; `groupId: null` after a reconnect */
  'network://group_changed': GroupsChanged
  'team://changed': TeamChanged
  'delegation://task_started': DelegationTaskStarted
  'delegation://task_completed': DelegationTaskCompleted
  /** Failed, cancelled, or timed out */
  'delegation://task_failed': DelegationTaskFailed
  /** A request you sent came back while nobody watched (an Inbox item) */
  'request://returned': RequestReturned
  /** A middleware stopped the run (turn limit, cost cap, ...) */
  'agent://turn_aborted': AgentTurnAborted
  /** The run ended as `compacted`; `newRunId` continues it */
  'agent://run_compacted': AgentRunCompacted
  /** `tool_search` added tools to the run */
  'agent://tools_loaded': AgentToolsLoaded
  // Local identity. Payloads are typed by the services that emit them
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
  'network://member_online': unknown
  'network://member_offline': unknown
  'network://membership_changed': unknown
  'identity://changed': unknown
  'team://roster_changed': unknown
  'member://status_changed': unknown
  'trust://relationship_updated': unknown
  /** Someone was refused for trust, or their request was answered (task 12) */
  'trust://request_received': { instanceId: string }
  // Capability events. Payloads are
  // typed by the services that emit them
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
  // Harness events. Payloads are
  // typed by the services that emit them
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
export type CoreEventName = Exclude<NandiEventName, 'nandi://resync'>

/**
 * Keyed by `CoreEventName` so the compiler rejects a core event that is
 * missing here, and one that is not in `NandiEventMap`. The desktop
 * client listens for exactly these names.
 */
const CORE_EVENTS: Record<CoreEventName, true> = {
  'agent://run': true,
  'agent://message': true,
  'agent://token': true,
  'agent://tool_call': true,
  'agent://tool_result': true,
  'agent://approval_request': true,
  'agent://approval_resolved': true,
  'agent://turn_done': true,
  'agent://agents_changed': true,
  'agent://warning': true,
  'agent://queue_changed': true,
  'dashboard://attention': true,
  'dashboard://activity': true,
  'workflow://step_started': true,
  'workflow://step_awaiting_approval': true,
  'workflow://step_done': true,
  'workflow://done': true,
  'workflow://failed': true,
  'workflow://changed': true,
  'trigger://fired': true,
  'trigger://rate_limited': true,
  'trigger://error': true,
  'trigger://changed': true,
  'marketplace://installed': true,
  'marketplace://uninstalled': true,
  'marketplace://forked': true,
  'marketplace://update_available': true,
  'marketplace://update_applied': true,
  'marketplace://error': true,
  'marketplace://progress': true,
  'repo://file_changed': true,
  'repo://validation_failed': true,
  'live://changed': true,
  'pilot://changed': true,
  'pilot://install_output': true,
  'pilot://consent_needed': true,
  'connector://changed': true,
  'connectors://changed': true,
  'grants://changed': true,
  'workspace://changed': true,
  'images://pull': true,
  'images://tools': true,
  'relay://status': true,
  'relay://agent_online': true,
  'relay://agent_offline': true,
  'relay://seal_refused': true,
  'network://member_key_changed': true,
  'network://group_changed': true,
  'team://changed': true,
  'delegation://task_started': true,
  'delegation://task_completed': true,
  'delegation://task_failed': true,
  'request://returned': true,
  'agent://turn_aborted': true,
  'agent://run_compacted': true,
  'agent://tools_loaded': true,
  'remote://status_changed': true,
  'remote://push_completed': true,
  'remote://pull_completed': true,
  'remote://error': true,
  'task://state_changed': true,
  'task://progress': true,
  'task://clarification_requested': true,
  'task://clarification_answered': true,
  'task://timed_out': true,
  'task://circuit_opened': true,
  'gateway://config_changed': true,
  'gateway://caller_updated': true,
  'gateway://rate_limited': true,
  'gateway://hold_changed': true,
  'gateway://assignment_changed': true,
  'network://member_online': true,
  'network://member_offline': true,
  'network://membership_changed': true,
  'identity://changed': true,
  'team://roster_changed': true,
  'member://status_changed': true,
  'trust://relationship_updated': true,
  'trust://request_received': true,
  'capability://created': true,
  'capability://updated': true,
  'capability://deleted': true,
  'capability://ownership_transferred': true,
  'provider://registered': true,
  'provider://unregistered': true,
  'registry://synced': true,
  'registry://sync_failed': true,
  'tool://catalog_changed': true,
  'tool://usage_recorded': true,
  'evidence://aggregated': true,
  'harness://detected': true,
  'harness://health_changed': true,
  'harness://session_started': true,
  'harness://session_ended': true,
  'harness://turn_delta': true,
  'harness://turn_finished': true,
  'harness://permission_request': true,
  'harness://error': true,
  'aws://profile_changed': true,
  'aws://credentials_expired': true,
  'bedrock://models_refreshed': true,
  'bedrock://gateway_status': true,
  'ask://streaming_delta': true,
  'ask://completed': true,
  'studio://artifact_loaded': true,
  'studio://artifact_saved': true,
  'studio://edit_previewed': true,
  'work://item_updated': true,
  'work://item_added': true,
  'work://item_removed': true,
  'request://updated': true,
  'network://member_joined': true,
  'network://member_left': true,
  'network://profile_updated': true,
  'task://reassigned': true,
  'workflow://manual_step_ready': true,
  'workflow://manual_step_completed': true,
  'config://repo_llm_changed': true,
  'run://plan_updated': true,
  'email://changed': true,
  'knowme://changed': true,
  'repo://rebuild_started': true,
  'repo://rebuild_completed': true,
}

export const CORE_EVENT_NAMES = Object.keys(CORE_EVENTS) as CoreEventName[]
