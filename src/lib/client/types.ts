/**
 * `NandiClient`: everything the UI can ask of the Nandi core, independent of
 * transport. Implemented over Tauri IPC (desktop) and HTTP/WebSocket
 * (browser). Methods return the same `Result` shape the Tauri bindings use,
 * so call sites handle errors identically on both.
 */

import type {
  AgentDefinitionInput,
  AgentPlaybooks,
  Playbook,
  PlaybookInput,
  AgentInfo,
  AgentOverview,
  AgentSnapshot,
  ApprovalDecision,
  AttachmentInfo,
  AttachmentKind,
  CompanionSnapshot,
  ConversationSnapshot,
  CreateAgentInput,
  CreateWorkflowInput,
  TemplateWorkflowInput,
  WorkflowTemplate,
  DashboardSnapshot,
  GitStatus,
  HandOverAnswer,
  JsonValue,
  RecoveryError,
  Result,
  RunDetail,
  RunPlan,
  SendMessageResult,
  StudioConversationSummary,
  Workflow,
  WorkflowSummary,
  PendingApproval,
} from '@/lib/bindings'
import type { commands } from '@/lib/bindings'
import type { NandiEvent } from './events'

/**
 * The generated commands. Ones declared with `rpc::commands!` in Rust work
 * on both transports as they are (see `invoke.ts`)
 */
type Commands = typeof commands

/**
 * The client: the generated commands, with the hand-written methods taking
 * the place of any command of the same name
 */
export type NandiClient = Omit<Commands, keyof HandWrittenClient> &
  HandWrittenClient

export interface Subscription {
  /**
   * Resolves once events are flowing. Load snapshots only after this, so no
   * event can slip between a snapshot read and the first listener.
   */
  ready: Promise<void>
  unsubscribe: () => void
}

/**
 * What each transport writes by hand: desktop-only calls, `subscribe`, and
 * commands not yet declared with `rpc::commands!`
 */
export interface HandWrittenClient {
  /** `tauri` (desktop webview) or `web` (browser) */
  readonly transport: 'tauri' | 'web'
  /** Features that need the desktop app */
  readonly supports: {
    /** Native file picker for attachments */
    attachments: boolean
  }

  // Agents
  listAgents(): Promise<Result<AgentInfo[], string>>
  /** Each agent's setup and last seven days; `todayStart`: local midnight */
  listAgentOverviews(
    todayStart: number
  ): Promise<Result<AgentOverview[], string>>
  createAgent(input: CreateAgentInput): Promise<Result<AgentInfo, string>>
  /** Starts the runtime container; returns its ID */
  startAgent(agentId: string): Promise<Result<string, string>>
  stopAgent(agentId: string): Promise<Result<null, string>>
  restartAgent(agentId: string): Promise<Result<string, string>>
  deleteAgent(agentId: string): Promise<Result<null, string>>

  // Chat
  sendMessage(
    agentId: string,
    text: string,
    attachments: string[],
    clientMessageId: string | null
  ): Promise<Result<SendMessageResult, string>>
  sendStudioMessage(
    conversationId: string,
    text: string,
    attachments: string[],
    clientMessageId: string | null
  ): Promise<Result<SendMessageResult, string>>
  createStudioConversation(): Promise<Result<StudioConversationSummary, string>>
  approveAction(
    agentId: string,
    approvalId: string,
    decision: ApprovalDecision
  ): Promise<Result<null, string>>
  /** Answers a browser hand-over (`pilots::handover`), whichever agent's */
  answerHandOver(
    approvalId: string,
    answer: HandOverAnswer
  ): Promise<Result<null, string>>
  /** Every agent's pending tool approvals, oldest first */
  listPendingApprovals(): Promise<Result<PendingApproval[], string>>
  cancelTurn(agentId: string, turnId: string): Promise<Result<null, string>>

  // Queue: one turn per agent; other requests wait, lowest priority first

  // Attachments (picking needs the desktop app; see `supports`)
  pickAttachments(
    kind: AttachmentKind
  ): Promise<Result<AttachmentInfo[], string>>
  discardAttachment(id: string): Promise<Result<null, string>>

  // Snapshots
  getAgentSnapshot(agentId: string): Promise<Result<AgentSnapshot, string>>
  /** `dayStart`: local midnight (ms), for "Runs Today" */
  getDashboardSnapshot(
    dayStart: number
  ): Promise<Result<DashboardSnapshot, string>>
  getRunDetail(runId: string): Promise<Result<RunDetail, string>>
  /** The host path of a file the run touched, `null` if it isn't there */
  resolveRunFile(
    runId: string,
    path: string
  ): Promise<Result<string | null, string>>
  /** The run's step-by-step plan, `null` when it has none */
  getRunPlan(runId: string): Promise<Result<RunPlan | null, string>>
  /**
   * One of the run's screenshots (`browser/<run>/…`) as a `data:` URL, for
   * its plan card; `null` once it's gone
   */
  getRunScreenshot(
    runId: string,
    path: string
  ): Promise<Result<string | null, string>>
  /** Opens it (or shows it in the file manager); desktop only */
  openRunFile(
    runId: string,
    path: string,
    reveal: boolean
  ): Promise<Result<null, string>>
  getWorkflows(): Promise<Result<WorkflowSummary[], string>>
  getStudioConversations(): Promise<Result<StudioConversationSummary[], string>>
  getConversation(
    conversationId: string
  ): Promise<Result<ConversationSnapshot, string>>

  // Settings
  saveEmergencyData(
    filename: string,
    data: JsonValue
  ): Promise<Result<null, RecoveryError>>
  loadEmergencyData(filename: string): Promise<Result<JsonValue, RecoveryError>>
  cleanupOldRecoveryFiles(): Promise<Result<number, RecoveryError>>

  // Egress

  // Workflows (listing is `getWorkflows`)
  getWorkflow(id: string): Promise<Result<Workflow, string>>
  /** Saves a YAML workflow as `workflows/{id}.yaml` in the agents repo */
  createWorkflow(input: CreateWorkflowInput): Promise<Result<Workflow, string>>
  /** The bundled templates for "New workflow › From a template" */
  listWorkflowTemplates(): Promise<Result<WorkflowTemplate[], string>>
  /** Builds a template with the chosen agents and saves it */
  createWorkflowFromTemplate(
    input: TemplateWorkflowInput
  ): Promise<Result<Workflow, string>>
  /** Returns the new run's ID */
  runWorkflow(id: string): Promise<Result<string, string>>
  /** Resolves a step paused by `requires_approval` */
  approveWorkflowStep(
    runId: string,
    stepId: string,
    decision: ApprovalDecision
  ): Promise<Result<null, string>>

  // Triggers (rules that start runs on schedules, files, webhooks, devices,
  // and Nandi's own events). A secret is only ever returned on create,
  // update to a webhook/device source, or rotate.
  /** Where webhook/device requests go, e.g. `http://127.0.0.1:17463` */
  getWebOrigin(): Promise<string | null>

  // Marketplace: agents and packs from shared git repos. Installed ones
  // are read-only until forked; nothing updates without the user asking.

  // Agent editor + git (wave 1, Track B). Saves go to the gold-copy files.
  /** Writes the agent's files and returns the saved agent */
  saveAgent(
    agentId: string,
    input: AgentDefinitionInput
  ): Promise<Result<AgentInfo, string>>
  /**
   * Replaces the agent's instructions.md (the user's rules). An installed
   * agent's are kept as an override that updates don't touch.
   */
  saveAgentInstructions(
    agentId: string,
    instructions: string
  ): Promise<Result<null, string>>
  /** Back to an installed agent's shipped instructions; whether it had the user's own */
  resetAgentInstructions(agentId: string): Promise<Result<boolean, string>>
  /** The agent's playbooks (`skills/`), and whether they're read-only */
  agentPlaybooks(agentId: string): Promise<Result<AgentPlaybooks, string>>
  /**
   * Validates and writes one playbook. `previous`: its old name when edited
   * or renamed, `null` for a new one
   */
  saveAgentPlaybook(
    agentId: string,
    input: PlaybookInput,
    previous: string | null
  ): Promise<Result<Playbook, string>>
  deleteAgentPlaybook(
    agentId: string,
    name: string
  ): Promise<Result<null, string>>
  /** One entity as stored in its file (`agent`, `workflow`, `trigger`, …) */
  loadEntity(kind: string, id: string): Promise<Result<JsonValue, string>>
  gitStatus(): Promise<Result<GitStatus, string>>
  /** Unified diff of one file against `HEAD` */
  gitDiff(path: string): Promise<Result<string, string>>
  /** Commits what's staged; returns the commit hash */
  gitCommit(message: string): Promise<Result<string, string>>

  // Pilots: host-side MCP servers for OS control (wave 1, Track A)

  // Runs across every agent, and the audit logs (wave 1, Track C)

  // Runtime connectors: MCP servers + OAuth2 (wave 1, Track D)

  // Wave 2A: teams, sharing, and delegation

  // Agent cards + capability index (Track A)

  // Relay + presence (Track B)

  // Delegation (Track C)

  // Teams: a shared git registry (Track D)

  // Shared views + trust (Track E)

  // Wave 2B redesign

  // Run detail + analysis (Track A)

  // Workflow builder (Track B)

  // Marketplace (Track C)

  // Companion (Track D). Window control needs the desktop app.
  /** `dayStart`: local midnight, epoch ms */
  getCompanionSnapshot(
    dayStart: number
  ): Promise<Result<CompanionSnapshot, string>>
  toggleCompanionWindow(): Promise<Result<null, string>>
  /** Quits the desktop app (the companion's way out) */
  quitNandi(): Promise<void>
  /** Sizes the companion for `shape` and keeps it in its screen corner */
  placeCompanion(
    shape: 'collapsed' | 'expanded' | 'overlay'
  ): Promise<Result<null, string>>
  /** The bison on the desktop (desktop only) */
  setCompanionBubble(bubble: boolean): Promise<Result<null, string>>
  /** Start Nandi with only the bison (desktop only) */
  setCompanionStartBisonOnly(on: boolean): Promise<Result<null, string>>
  setCompanionPinned(pinned: boolean): Promise<Result<null, string>>
  /** `scopeRequestId` pins the conversation to a Request (`null` clears it) */
  sendCompanionMessage(
    text: string,
    conversationId: string | null,
    scopeRequestId: string | null
  ): Promise<Result<SendMessageResult, string>>
  /** "Notify me" (OS notifications for approvals and questions) */
  setCompanionNotify(notify: boolean): Promise<Result<null, string>>
  /** "Notify me when requests come back" */
  setCompanionNotifyResults(notify: boolean): Promise<Result<null, string>>
  /** Watch in companion: shows it with the Request pinned (desktop only) */
  companionFocusRequest(requestId: string): Promise<Result<null, string>>

  // Command palette (Track E)

  // Wave final

  // Local-identity freeze

  // Identity: display name + user slug in `~/.nandi/repo/NANDI.md`

  // GitHub sign-in: one token for every github.com repository

  // Optional git remote for the agents repo

  // Task lifecycle

  // Clarification

  // Gateway agent
  // Live UI (`services::live`)
  // Email (the Chief of Staff reads it to keep My notes)
  // Connector hub: grants, registry connectors, calendars and files

  // Network directory

  // Team roster, member profiles, and per-member trust

  // Groups (protocol 7, `services::groups`)

  // Wave A: capabilities

  // Wave A: the unified tool catalog

  // Wave A: capability registries (not the marketplace registries)

  // Wave A: capability presence on the relay

  // Wave B: harness adapters

  // Wave B: AWS auth, read from the user's own AWS config (never key material)
  /** `aws sso login` (or `aws login` for a console profile): the AWS CLI
   * opens the browser; poll for the outcome */

  // Wave B: Bedrock models and the LiteLLM gateway

  // Wave CD: the ask box (see docs/developer/interaction-layer.md)

  // Wave CD: Studio's artifact editor

  // Wave CD: work in flight

  // Wave EF: supervisor reassignment
  // Wave EF: manual workflow steps
  // Wave EF: `.nandi/llm.yaml`

  /** Opens `url` in the system browser */
  openExternal(url: string): Promise<void>

  /**
   * Receives every core event. Call once at boot. The web client also
   * emits `nandi://resync` after reconnecting.
   */
  subscribe(handler: (event: NandiEvent) => void): Subscription
}
