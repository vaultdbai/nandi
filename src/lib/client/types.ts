/**
 * `NandiClient`: everything the UI can ask of the Nandi core, independent of
 * transport. Implemented over Tauri IPC (desktop) and HTTP/WebSocket
 * (browser). Methods return the same `Result` shape the Tauri bindings use,
 * so call sites handle errors identically on both.
 */

import type {
  AddRegistryInput,
  AgentCard,
  AgentConnectorChoice,
  FolderUsers,
  AgentDefinitionInput,
  AgentPlaybooks,
  Playbook,
  PlaybookInput,
  AgentDetail,
  AgentInfo,
  AgentOverview,
  AgentManifest,
  AppliedUpdate,
  AgentSnapshot,
  ApprovalDecision,
  AttachmentInfo,
  AttachmentKind,
  CommunityRegistry,
  CompanionSnapshot,
  ConnectorInfo,
  ConnectorStatus,
  ConnectorConsent,
  ConnectorEntry,
  PublishConnectorInput,
  PublishConnectorResult,
  ScaffoldConnectorInput,
  ConsentStatus,
  ConversationSnapshot,
  CreateAgentInput,
  CreateWorkflowInput,
  TemplateWorkflowInput,
  WorkflowTemplate,
  DashboardSnapshot,
  ForkedAgent,
  GitStatus,
  HandOverAnswer,
  InstalledAgentInfo,
  InstalledItem,
  InstalledPackInfo,
  JsonValue,
  KeyedProvider,
  LlmKeyStatus,
  LlmKeyTest,
  DefaultModelStatus,
  TrialAccount,
  OllamaStatus,
  MarketplaceFilter,
  PackDetail,
  PackManifest,
  PublishAgentInput,
  PublishResult,
  RecoveryError,
  RegistryInfo,
  RelayPeer,
  RelayStatus,
  JoinOutcome,
  PublicRelayAccount,
  PublicRelayJoin,
  PublicRelayOffer,
  RelayInvite,
  RepoStatus,
  Result,
  RunDetail,
  RunPlan,
  SendMessageResult,
  StudioConversationSummary,
  UpdateAvailable,
  Workflow,
  WorkflowSummary,
  CallerPolicy,
  GatewayConfig,
  GatewayManifest,
  GatewayStatus,
  GatewayAssignment,
  GatewayHold,
  GatewayUsage,
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
  getLlmKeyStatus(): Promise<Result<LlmKeyStatus[], string>>
  /** The default model, and the one calls use with the keys set now */
  getDefaultModel(): Promise<Result<DefaultModelStatus, string>>
  /** `<provider>:<model>`, e.g. `claude:claude-sonnet-5` */
  setDefaultModel(model: string): Promise<Result<DefaultModelStatus, string>>
  /** Turns the free trial on (its model becomes the default) or off */
  setFreeTrial(enabled: boolean): Promise<Result<DefaultModelStatus, string>>
  /** The free trial's allowance as Nandi's trial gateway counts it */
  getFreeTrialAccount(): Promise<Result<TrialAccount, string>>
  /** Starts linking this computer's free trial to a person's account */
  /** Checks a key (`null`: the stored one) without storing it */
  testLlmKey(
    provider: KeyedProvider,
    apiKey: string | null
  ): Promise<Result<LlmKeyTest, string>>
  /** The provider's current models (its stored key), newest first */
  listProviderModels(provider: KeyedProvider): Promise<Result<string[], string>>
  /** Whether Ollama runs on this computer, and its models */
  detectOllama(): Promise<Result<OllamaStatus, string>>
  setLlmApiKey(
    provider: KeyedProvider,
    apiKey: string
  ): Promise<Result<null, string>>
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
  /** Every pack in the configured registries (starter packs) */
  listAvailablePacks(): Promise<Result<PackManifest[], string>>
  listMarketplacePacks(
    filter?: Partial<MarketplaceFilter>
  ): Promise<Result<PackManifest[], string>>
  listAvailableAgents(
    filter?: Partial<MarketplaceFilter>
  ): Promise<Result<AgentManifest[], string>>
  /** README, versions, capability breakdown, publisher */
  getAgentDetail(agentId: string): Promise<Result<AgentDetail, string>>
  /** README and versions */
  getPackDetail(packId: string): Promise<Result<PackDetail, string>>
  installPack(packId: string): Promise<Result<InstalledPackInfo, string>>
  /** What a pack in a local folder or `.zip` would install (for consent) */
  inspectLocalPack(path: string): Promise<Result<PackManifest, string>>
  /** Installs a pack from a local folder or `.zip` (unverified, offline) */
  installLocalPack(path: string): Promise<Result<InstalledPackInfo, string>>
  installAgent(agentId: string): Promise<Result<InstalledAgentInfo, string>>
  listInstalled(): Promise<Result<InstalledItem[], string>>
  /** `kind`: `pack` | `agent`. Refused for forked agents. */
  uninstallInstalled(kind: string, id: string): Promise<Result<null, string>>
  forkInstalledAgent(
    installedId: string,
    newId: string,
    newName: string
  ): Promise<Result<ForkedAgent, string>>
  /** Reports newer versions; never applies them */
  checkForUpdates(): Promise<Result<UpdateAvailable[], string>>
  applyUpdate(
    installedId: string,
    newRef: string
  ): Promise<Result<AppliedUpdate, string>>
  /** What a marketplace agent requests, and what the user approved */
  getAgentConsent(agentId: string): Promise<Result<ConsentStatus, string>>
  grantAgentConsent(
    agentId: string,
    categories: string[]
  ): Promise<Result<ConsentStatus, string>>
  listRegistries(): Promise<Result<RegistryInfo[], string>>
  addRegistry(input: AddRegistryInput): Promise<Result<RegistryInfo, string>>
  removeRegistry(url: string): Promise<Result<null, string>>
  /** A private `nandi-community-<login>` repo, added to the registries */
  createCommunityRegistry(): Promise<Result<CommunityRegistry, string>>
  /** Tags the synced agent and opens a registry pull request */
  publishAgent(input: PublishAgentInput): Promise<Result<PublishResult, string>>

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
  /** The local agents repo (`~/.nandi/repo`): root, counts, problems */
  repoStatus(): Promise<Result<RepoStatus, string>>
  gitStatus(): Promise<Result<GitStatus, string>>
  /** Unified diff of one file against `HEAD` */
  gitDiff(path: string): Promise<Result<string, string>>
  /** Commits what's staged; returns the commit hash */
  gitCommit(message: string): Promise<Result<string, string>>

  // Pilots: host-side MCP servers for OS control (wave 1, Track A)

  // Runs across every agent, and the audit logs (wave 1, Track C)

  // Runtime connectors: MCP servers + OAuth2 (wave 1, Track D)
  listConnectors(): Promise<Result<ConnectorInfo[], string>>
  installConnector(id: string): Promise<Result<null, string>>
  startConnector(id: string): Promise<Result<null, string>>
  stopConnector(id: string): Promise<Result<null, string>>
  /** Replaces an installed connector's settings (never secrets) */
  configureConnector(
    id: string,
    config: Partial<Record<string, JsonValue>>
  ): Promise<Result<null, string>>
  /** The connectors an agent uses, with its shared folders */
  getAgentConnectors(
    agentId: string
  ): Promise<Result<AgentConnectorChoice[], string>>
  setAgentConnectors(
    agentId: string,
    connectors: AgentConnectorChoice[]
  ): Promise<Result<null, string>>
  /** Saves a secret in the OS keyring; empty removes it. Write only */
  setConnectorSecret(
    id: string,
    key: string,
    value: string
  ): Promise<Result<null, string>>
  connectorStatus(id: string): Promise<Result<ConnectorStatus, string>>

  // Wave 2A: teams, sharing, and delegation

  // Agent cards + capability index (Track A)

  // Relay + presence (Track B)
  getRelayStatus(): Promise<Result<RelayStatus, string>>
  /** Saves the relay URL and reconnects (empty: Disconnect) */
  setRelayUrl(url: string): Promise<Result<null, string>>
  /**
   * "Join": joins the relay at `url`, enrolling with `joinCode` (or the
   * build's own code for the build's relay) when it is secured
   */
  joinRelay(
    url: string,
    joinCode: string | null
  ): Promise<Result<JoinOutcome, string>>
  /** What "Copy invite message" passes on (`null`: no relay in use) */
  getRelayInvite(): Promise<Result<RelayInvite | null, string>>
  /** "Join the public relay": whether it can be joined now, and as whom */
  getPublicRelayOffer(): Promise<Result<PublicRelayOffer, string>>
  /**
   * Joins the public relay with this Nandi's connected email; on `joined`
   * the core keeps the token and connects
   */
  joinPublicRelay(): Promise<Result<PublicRelayJoin, string>>
  /** The public-relay account this Nandi joined as */
  getPublicRelayAccount(): Promise<Result<PublicRelayAccount, string>>
  /** Agents online at the relay, optionally only those serving `capability` */
  listOnlineAgents(
    capability: string | null
  ): Promise<Result<AgentCard[], string>>
  /**
   * "Test connection": WebSocket ping to `url`; the round trip in ms, or
   * the exact error
   */
  testRelayConnection(url: string): Promise<Result<number, string>>
  /** The instances at the relay, from its `/instances` endpoint */
  getRelayInstances(): Promise<Result<RelayPeer[], string>>

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
  /** Which agents get each of a filesystem connector's shared folders */
  listFolderUsers(connectorId: string): Promise<Result<FolderUsers[], string>>
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
  getGatewayStatus(): Promise<Result<GatewayStatus, string>>
  /** Turns on relay.enabled + discoverable and re-registers the card */
  makeGatewayReachable(): Promise<Result<GatewayStatus, string>>
  /** Requests waiting for the owner's approval, oldest first */
  listGatewayHolds(): Promise<Result<GatewayHold[], string>>
  /** Approve (routing goes on) or decline a held request; `note` goes back to them */
  decideGatewayHold(
    taskId: string,
    approve: boolean,
    note: string | null
  ): Promise<Result<GatewayHold, string>>
  // Live UI (`services::live`)
  // Email (the Chief of Staff reads it to keep My notes)
  // Connector hub: grants, registry connectors, calendars and files
  /** Connectors listed in the configured registries */
  listRegistryConnectors(
    refresh: boolean
  ): Promise<Result<ConnectorEntry[], string>>
  getConnectorConsent(
    registryUrl: string,
    connectorId: string
  ): Promise<Result<ConnectorConsent, string>>
  installRegistryConnector(
    registryUrl: string,
    connectorId: string,
    apiKey: string | null
  ): Promise<Result<null, string>>
  removeConnector(id: string): Promise<Result<null, string>>
  scaffoldConnector(
    input: ScaffoldConnectorInput
  ): Promise<Result<ConnectorInfo, string>>
  publishConnector(
    input: PublishConnectorInput
  ): Promise<Result<PublishConnectorResult, string>>
  /** Work assigned to the owner that's still open, soonest due first */
  listGatewayAssignments(): Promise<Result<GatewayAssignment[], string>>
  /** Done (the requester gets the note) or Decline an assignment */
  closeGatewayAssignment(
    taskId: string,
    done: boolean,
    note: string | null
  ): Promise<Result<GatewayAssignment, string>>
  getGatewayConfig(): Promise<Result<GatewayConfig, string>>
  setGatewayConfig(
    config: GatewayConfig
  ): Promise<Result<GatewayConfig, string>>
  listGatewayCallers(): Promise<Result<CallerPolicy[], string>>
  setCallerPolicy(policy: CallerPolicy): Promise<Result<CallerPolicy, string>>
  removeCallerPolicy(callerInstance: string): Promise<Result<null, string>>
  getGatewayUsage(callerInstance: string): Promise<Result<GatewayUsage, string>>
  getGatewayManifest(): Promise<Result<GatewayManifest, string>>
  setGatewayManifest(
    manifest: GatewayManifest
  ): Promise<Result<GatewayManifest, string>>
  /** Ask the gateway about this instance; it answers, never runs work */
  gatewayChat(message: string): Promise<Result<string, string>>

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
