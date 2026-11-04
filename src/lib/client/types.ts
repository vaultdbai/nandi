/**
 * `NandiClient`: everything the UI can ask of the Nandi core, independent of
 * transport. Implemented over Tauri IPC (desktop) and HTTP/WebSocket
 * (browser). Methods return the same `Result` shape the Tauri bindings use,
 * so call sites handle errors identically on both.
 */

import type {
  RelayTrust,
  RequestReturn,
  ActionPlan,
  MascotInfo,
  PhraseResponse,
  AddRegistryInput,
  AgentCard,
  AgentConnectorChoice,
  FolderUsers,
  ImageStatus,
  SandboxImage,
  AgentDefinitionInput,
  AgentDelegationSettings,
  AgentDetail,
  AgentInfo,
  AgentOverview,
  AgentManifest,
  AgentPilotPermission,
  AgentPolicies,
  AnalysisResponse,
  AppliedUpdate,
  AgentSnapshot,
  AppPreferences,
  ApprovalDecision,
  AttachmentInfo,
  AttachmentKind,
  AuditEntry,
  CommunityRegistry,
  CompanionSnapshot,
  ConnectorInfo,
  ConnectorStatus,
  ConsentStatus,
  ConversationSnapshot,
  CreateAgentInput,
  CreateWorkflowInput,
  CalendarPreview,
  CronPreview,
  TemplateWorkflowInput,
  WorkflowTemplate,
  DashboardSnapshot,
  DateRange,
  DeviceFlowState,
  EgressStatus,
  ForkedAgent,
  GitStatus,
  InstalledAgentInfo,
  InstalledItem,
  InstalledPackInfo,
  JsonValue,
  KeyedProvider,
  LlmKeyStatus,
  LlmKeyTest,
  DefaultModelStatus,
  OllamaStatus,
  OnboardingStep,
  MarketplaceFilter,
  MarketplaceSearchHit,
  MemoryEntry,
  PackDetail,
  PackManifest,
  PaletteActionOutcome,
  PaletteResult,
  PilotDefinition,
  PilotStatus,
  PolicySummary,
  PublishAgentInput,
  PublishResult,
  QueueEntry,
  RecoveryError,
  RegistryInfo,
  RelayPeer,
  RelayStatus,
  JoinOutcome,
  RelayInvite,
  RepoStatus,
  Result,
  RunDetail,
  RunFilters,
  RunPage,
  RunStats,
  RunSummary,
  SavedTrigger,
  SendMessageResult,
  SharedAgent,
  CapabilityProviders,
  StudioConversationSummary,
  TaskResult,
  Team,
  TeamDetail,
  Trigger,
  TriggerFire,
  TriggerInput,
  TrustPolicy,
  TrustRule,
  UpdateAvailable,
  Workflow,
  WorkflowGraph,
  WorkflowSummary,
  CallerPolicy,
  ClarificationPolicy,
  ClarificationResponse,
  EtaEstimate,
  GatewayConfig,
  GatewayManifest,
  GatewayStatus,
  GatewayAssignment,
  MyNotes,
  NoteFile,
  GatewayHold,
  GatewayUsage,
  IdentityStatus,
  GithubSignIn,
  MemberProfile,
  PeerKeyStatus,
  NetworkSnapshot,
  PendingClarification,
  PendingApproval,
  PullResult,
  PushResult,
  RemoteCredential,
  RemoteStatus,
  TaskEvent,
  TaskFilter,
  TaskMessage,
  TaskRecord,
  Request,
  TaskStatusReport,
  TeamRoster,
  TrustRelationship,
  TrustUpdate,
  TrustRequest,
  TrustAnswer,
  ShareResult,
  AmbiguityCluster,
  CapabilityDefinition,
  CapabilityEvidence,
  CapabilityListing,
  CapabilityOwner,
  CapabilityProvider,
  CapabilityProviderStats,
  CapabilityRegistry,
  InstancePresence,
  RegistryKind,
  SyncResult,
  ToolEntry,
  ToolUsageStats,
  AwsCliInfo,
  AwsProfile,
  KeyProfileInput,
  SsoSetup,
  SsoSetupFinish,
  SsoSetupInput,
  BedrockApiKeyStatus,
  BedrockModel,
  CredentialStatus,
  HarnessDefault,
  HarnessEventRecord,
  HarnessHealth,
  HarnessInfo,
  HarnessSessionSummary,
  HarnessTestResult,
  LitellmGatewayStatus,
  SsoLoginResult,
  ArtifactLoad,
  ArtifactRef,
  ArtifactSave,
  ArtifactSaveResult,
  AskHistoryEntry,
  AskRequest,
  AskResponse,
  EditPreview,
  PushTargetOption,
  ArtifactValidation,
  WorkItem,
  WorkState,
  ReassignRequest,
  ReassignCandidate,
  ManualStepCompletion,
  PendingManualStep,
  RepoLlmConfigStatus,
} from '@/lib/bindings'
import type { NandiEvent } from './events'

export interface Subscription {
  /**
   * Resolves once events are flowing. Load snapshots only after this, so no
   * event can slip between a snapshot read and the first listener.
   */
  ready: Promise<void>
  unsubscribe: () => void
}

export interface NandiClient {
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
  /** Every agent's pending tool approvals, oldest first */
  listPendingApprovals(): Promise<Result<PendingApproval[], string>>
  cancelTurn(agentId: string, turnId: string): Promise<Result<null, string>>

  // Queue: one turn per agent; other requests wait, lowest priority first
  getAgentQueue(agentId: string): Promise<Result<QueueEntry[], string>>
  /** The running or approval-blocked turn, or `null` when idle */
  getCurrentTurn(agentId: string): Promise<Result<RunSummary | null, string>>
  /** Removes a queued run (never the current turn; see `cancelTurn`) */
  cancelQueuedRun(runId: string): Promise<Result<null, string>>
  /** Lower priorities run sooner */
  reprioritizeQueuedRun(
    runId: string,
    newPriority: number
  ): Promise<Result<null, string>>

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
  loadPreferences(): Promise<Result<AppPreferences, string>>
  savePreferences(preferences: AppPreferences): Promise<Result<null, string>>
  getLlmKeyStatus(): Promise<Result<LlmKeyStatus[], string>>
  /** The default model, and the one calls use with the keys set now */
  getDefaultModel(): Promise<Result<DefaultModelStatus, string>>
  /** `<provider>:<model>`, e.g. `claude:claude-sonnet-5` */
  setDefaultModel(model: string): Promise<Result<DefaultModelStatus, string>>
  /** Turns the free trial on (its model becomes the default) or off */
  setFreeTrial(enabled: boolean): Promise<Result<DefaultModelStatus, string>>
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
  getEgressStatus(): Promise<Result<EgressStatus, string>>
  /** Empty `accessKey` and `secret` remove the stored key */
  setAwsCredentials(
    accessKey: string,
    secret: string
  ): Promise<Result<null, string>>
  setAwsRegion(region: string): Promise<Result<null, string>>

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
  listTriggers(): Promise<Result<Trigger[], string>>
  getTrigger(id: string): Promise<Result<Trigger, string>>
  createTrigger(input: TriggerInput): Promise<Result<SavedTrigger, string>>
  updateTrigger(
    id: string,
    input: TriggerInput
  ): Promise<Result<SavedTrigger, string>>
  deleteTrigger(id: string): Promise<Result<null, string>>
  setTriggerEnabled(id: string, enabled: boolean): Promise<Result<null, string>>
  rotateTriggerSecret(id: string): Promise<Result<SavedTrigger, string>>
  /** Fires by hand (even if disabled); `null` sends `{"test": true}` */
  testFireTrigger(
    id: string,
    payload: unknown
  ): Promise<Result<TriggerFire, string>>
  listTriggerFires(
    triggerId: string,
    limit: number | null
  ): Promise<Result<TriggerFire[], string>>
  /** Where webhook/device requests go, e.g. `http://127.0.0.1:17463` */
  getWebOrigin(): Promise<string | null>
  /** Whether a cron schedule is valid, and its next 3 runs */
  previewCron(schedule: string): Promise<Result<CronPreview, string>>
  /** Whether a calendar config is valid, and its next 5 runs with who is on duty */
  previewCalendar(
    config: Record<string, unknown>
  ): Promise<Result<CalendarPreview, string>>

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
  listPilots(): Promise<Result<PilotDefinition[], string>>
  installPilot(id: string): Promise<Result<null, string>>
  /** Returns the pilot's process ID */
  startPilot(id: string): Promise<Result<number, string>>
  stopPilot(id: string): Promise<Result<null, string>>
  pilotStatus(id: string): Promise<Result<PilotStatus, string>>
  grantPilotCapability(
    agentId: string,
    pilotId: string,
    capability: string
  ): Promise<Result<null, string>>
  revokePilotCapability(
    agentId: string,
    pilotId: string,
    capability: string
  ): Promise<Result<null, string>>
  /** Saves a pilot secret the agent's skill.md names (empty removes it) */
  setPilotSecret(
    agentId: string,
    pilotId: string,
    name: string,
    value: string
  ): Promise<Result<null, string>>
  /** The pilots the agent declares, and what the user granted each */
  agentPilotPermissions(
    agentId: string
  ): Promise<Result<AgentPilotPermission[], string>>
  /** The policies over an agent and what each changed, or why it can't run */
  agentPolicies(agentId: string): Promise<Result<AgentPolicies, string>>
  /** Every policy in the repo by id and name, broken ones included */
  listPolicies(): Promise<Result<PolicySummary[], string>>

  // Runs across every agent, and the audit logs (wave 1, Track C)
  /** Newest first */
  listAllRuns(
    filters: RunFilters,
    limit: number,
    offset: number
  ): Promise<Result<RunPage, string>>
  getRunStats(range: DateRange): Promise<Result<RunStats, string>>
  /** `kind`: `llm` | `egress` | `triggers` | `pilots` | `connectors`; newest first */
  listAuditLog(
    kind: string,
    limit: number,
    offset: number
  ): Promise<Result<AuditEntry[], string>>

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
  /** Starts an OAuth2 device flow; show `userCode` and `verificationUri` */
  startOauth(connectorId: string): Promise<Result<DeviceFlowState, string>>
  /** Polls once; `true` once authorized */
  pollOauth(connectorId: string): Promise<Result<boolean, string>>

  // Wave 2A: teams, sharing, and delegation

  // Agent cards + capability index (Track A)
  /** Local agents serving `capability` */
  findAgentsByCapability(
    capability: string
  ): Promise<Result<AgentCard[], string>>
  /** Every capability a local agent serves */
  listDeclaredCapabilities(): Promise<Result<string[], string>>
  getAgentCard(agentId: string): Promise<Result<AgentCard, string>>

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
  /** Local and remote agents serving `capability` */
  listAvailableProviders(
    capability: string
  ): Promise<Result<AgentCard[], string>>
  /** Starts a delegation; returns the task ID. `target` (a card ID) skips scoring */
  delegateTask(
    capability: string,
    input: JsonValue,
    target: string | null
  ): Promise<Result<string, string>>
  /** Newest first, both directions */
  listDelegationHistory(limit: number): Promise<Result<TaskResult[], string>>
  getAgentDelegation(
    agentId: string
  ): Promise<Result<AgentDelegationSettings, string>>
  setAgentDelegation(
    agentId: string,
    settings: AgentDelegationSettings
  ): Promise<Result<null, string>>

  // Teams: a shared git registry (Track D)
  listTeams(): Promise<Result<Team[], string>>
  getTeam(teamId: string): Promise<Result<TeamDetail, string>>
  /** `registryUrl`: the team registry's git URL */
  createTeam(registryUrl: string): Promise<Result<Team, string>>
  joinTeam(teamId: string): Promise<Result<null, string>>
  leaveTeam(teamId: string): Promise<Result<null, string>>
  syncTeamRegistry(teamId: string): Promise<Result<null, string>>
  /** Pushes a share branch; a team admin merges it */
  shareAgent(
    teamId: string,
    agentId: string
  ): Promise<Result<ShareResult, string>>
  unshareAgent(teamId: string, agentId: string): Promise<Result<null, string>>

  // Shared views + trust (Track E)
  /** From my teams' registries and the relay */
  listAgentsSharedWithMe(): Promise<Result<SharedAgent[], string>>
  /** Mine that accept delegation or are shared to a team */
  listAgentsSharedByMe(): Promise<Result<SharedAgent[], string>>
  /** Per capability: who serves it, ranked, and my pin (`null`: all) */
  listCapabilityProviders(
    capability: string | null
  ): Promise<Result<CapabilityProviders[], string>>
  /** Pins a provider card for a capability; `null` unpins */
  setProviderPin(
    capability: string,
    cardId: string | null
  ): Promise<Result<null, string>>
  listTrustPolicies(): Promise<Result<TrustRule[], string>>
  /** `requester`: a card ID or `*`; `providerAgent`: a local agent ID */
  setTrustPolicy(
    requester: string,
    providerAgent: string,
    policy: TrustPolicy
  ): Promise<Result<null, string>>

  // Wave 2B redesign

  // Run detail + analysis (Track A)
  /** Answers `question` from the run's own record, citing sources */
  analyzeRun(
    runId: string,
    question: string
  ): Promise<Result<AnalysisResponse, string>>
  /** A short markdown summary of the run (cached) */
  summarizeRun(runId: string): Promise<Result<string, string>>

  // Workflow builder (Track B)
  getWorkflowGraph(workflowId: string): Promise<Result<WorkflowGraph, string>>
  /** Saves the workflow's YAML; returns the reloaded workflow */
  saveWorkflowYaml(
    workflowId: string,
    yaml: string
  ): Promise<Result<Workflow, string>>

  // Marketplace (Track C)
  searchMarketplace(
    query: string
  ): Promise<Result<MarketplaceSearchHit[], string>>

  // Companion (Track D). Window control needs the desktop app.
  /** `dayStart`: local midnight, epoch ms */
  getCompanionSnapshot(
    dayStart: number
  ): Promise<Result<CompanionSnapshot, string>>
  toggleCompanionWindow(): Promise<Result<null, string>>
  /** Quits the desktop app (the companion's way out) */
  quitNandi(): Promise<void>
  /** A sandbox image: built, Docker running, a build's progress */
  getImageStatus(image: SandboxImage): Promise<Result<ImageStatus, string>>
  /** Builds it in the background (`images://build` events) */
  buildImage(image: SandboxImage): Promise<Result<null, string>>
  /** Downloads the pinned image in the background (`images://build` events) */
  pullImage(image: SandboxImage): Promise<Result<null, string>>
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
  /** `>`-prefixed queries search actions only */
  searchPalette(query: string): Promise<Result<PaletteResult[], string>>
  executePaletteAction(
    actionId: string,
    args: JsonValue
  ): Promise<Result<PaletteActionOutcome, string>>

  // Wave final

  // Agent memory (Track C)
  loadAgentMemory(agentId: string): Promise<Result<MemoryEntry[], string>>
  updateAgentMemory(
    agentId: string,
    index: number,
    content: string
  ): Promise<Result<null, string>>
  deleteAgentMemory(
    agentId: string,
    index: number
  ): Promise<Result<null, string>>
  /** Re-extracts from the latest run; returns the whole memory */
  refreshAgentMemory(agentId: string): Promise<Result<MemoryEntry[], string>>
  getAgentMemoryEnabled(agentId: string): Promise<Result<boolean, string>>
  setAgentMemoryEnabled(
    agentId: string,
    enabled: boolean
  ): Promise<Result<null, string>>

  // Local-identity freeze

  // Identity: display name + user slug in `~/.nandi/repo/NANDI.md`
  getIdentityStatus(): Promise<Result<IdentityStatus, string>>
  setDisplayName(name: string): Promise<Result<null, string>>
  setUserSlug(slug: string): Promise<Result<null, string>>
  /** Where first-run setup is, so a relaunch resumes there */
  setOnboardingStep(step: OnboardingStep): Promise<Result<null, string>>

  // GitHub sign-in: one token for every github.com repository
  getGithubSignIn(): Promise<Result<GithubSignIn, string>>
  /** Show `userCode` and open `verificationUri` */
  startGithubSignIn(): Promise<Result<DeviceFlowState, string>>
  /** Asks GitHub once; `signedIn` once the user approved */
  pollGithubSignIn(): Promise<Result<GithubSignIn, string>>
  signOutGithub(): Promise<Result<GithubSignIn, string>>

  // Optional git remote for the agents repo
  getRemoteStatus(): Promise<Result<RemoteStatus, string>>
  configureRemote(
    url: string,
    credential: RemoteCredential | null,
    prefix: string | null,
    slug: string | null
  ): Promise<Result<RemoteStatus, string>>
  clearRemote(): Promise<Result<null, string>>
  testRemoteConnection(): Promise<Result<null, string>>
  pushToRemote(overwrite: boolean): Promise<Result<PushResult, string>>
  pullFromRemote(): Promise<Result<PullResult, string>>
  listRemoteBranches(): Promise<Result<string[], string>>

  // Task lifecycle
  getTask(taskId: string): Promise<Result<TaskRecord, string>>
  listTasks(filter: TaskFilter | null): Promise<Result<TaskRecord[], string>>
  cancelTask(
    taskId: string,
    reason: string | null
  ): Promise<Result<TaskRecord, string>>
  queryTaskStatus(taskId: string): Promise<Result<TaskStatusReport, string>>
  getTaskEvents(taskId: string): Promise<Result<TaskEvent[], string>>
  getTaskMessages(taskId: string): Promise<Result<TaskMessage[], string>>
  getEtaEstimate(
    capability: string,
    providerAgent: string | null
  ): Promise<Result<EtaEstimate, string>>

  // Requests (v1): one unit of work with its Steps; `id` is any task ID in it
  getRequest(id: string): Promise<Result<Request, string>>
  /** The Request a run is part of, if any */
  requestForRun(runId: string): Promise<Result<string | null, string>>
  /** A failed outgoing Request on the next provider (attempt n + 1) */
  retryTask(id: string): Promise<Result<Request, string>>
  /** Saves an ask's approved plan as a Request and starts it (its ID) */
  startRequest(plan: ActionPlan, text: string): Promise<Result<string, string>>
  /** Marks a step of a local Request that waits on the user done */
  completeRequestStep(
    id: string,
    position: number
  ): Promise<Result<null, string>>
  /** Stops a local Request and what its steps started */
  cancelRequest(id: string): Promise<Result<null, string>>
  // The mascot (Nandi rename)
  /** The bull's name */
  mascotGet(): Promise<Result<MascotInfo, string>>
  /** Renames the bull (`null`: back to Nandi) */
  mascotSet(name: string | null): Promise<Result<MascotInfo, string>>
  /** The bull's reply to the hidden phrase (`null` for anything else) */
  mascotPhrase(input: string): Promise<Result<PhraseResponse | null, string>>
  /** Do this again: the same plan as a new Request (its ID) */
  rerunRequest(id: string): Promise<Result<string, string>>
  /** Save as workflow: a workflow file drafted from a Request's plan */
  draftWorkflowFromRequest(id: string): Promise<Result<string, string>>

  // Clarification
  respondToClarification(
    response: ClarificationResponse
  ): Promise<Result<null, string>>
  escalateClarification(
    taskId: string,
    questionId: string
  ): Promise<Result<null, string>>
  getPendingClarifications(): Promise<Result<PendingClarification[], string>>
  /** The requester agent's policy (skill.md `clarification:`) */
  getClarificationPolicy(
    agentId: string
  ): Promise<Result<ClarificationPolicy, string>>
  /** Returns the policy as saved (bounded, topics de-duplicated) */
  setClarificationPolicy(
    agentId: string,
    policy: ClarificationPolicy
  ): Promise<Result<ClarificationPolicy, string>>

  // Gateway agent
  getGatewayStatus(): Promise<Result<GatewayStatus, string>>
  /** Turns on relay.enabled + discoverable and re-registers the card */
  makeGatewayReachable(): Promise<Result<GatewayStatus, string>>
  /** Requests waiting for the owner's approval, oldest first */
  listGatewayHolds(): Promise<Result<GatewayHold[], string>>
  /** Approve (routing goes on) or decline a held request */
  decideGatewayHold(
    taskId: string,
    approve: boolean
  ): Promise<Result<GatewayHold, string>>
  // My notes (the folder Chief of Staff keeps)
  /** The notes folder and its notes; creates and shares it on first use */
  getMyNotes(): Promise<Result<MyNotes, string>>
  readNote(name: string): Promise<Result<string, string>>
  saveNote(name: string, content: string): Promise<Result<NoteFile, string>>
  deleteNote(name: string): Promise<Result<null, string>>
  /** Lets agents update the notes (without asking each time), or not */
  setNotesAgentsCanWrite(allow: boolean): Promise<Result<MyNotes, string>>
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
  getNetworkSnapshot(): Promise<Result<NetworkSnapshot, string>>
  refreshNetwork(): Promise<Result<NetworkSnapshot, string>>

  // Team roster, member profiles, and per-member trust
  getTeamRoster(teamId: string): Promise<Result<TeamRoster, string>>
  getMemberProfile(
    teamId: string,
    handle: string
  ): Promise<Result<MemberProfile, string>>
  getMemberByInstance(
    instanceId: string
  ): Promise<Result<MemberProfile, string>>
  /** Their key changed: pin the new one, so sending to them works again */
  acceptPeerKey(instanceId: string): Promise<Result<PeerKeyStatus, string>>
  setTrustRelationship(
    instanceId: string,
    update: TrustUpdate
  ): Promise<Result<TrustRelationship, string>>
  /** "Trust everyone on my company's relay", and whether it applies */
  getRelayTrust(): Promise<Result<RelayTrust, string>>
  setRelayTrust(enabled: boolean): Promise<Result<RelayTrust, string>>
  blockAgentFromMember(
    instanceId: string,
    agentId: string
  ): Promise<Result<TrustRelationship, string>>
  unblockAgentFromMember(
    instanceId: string,
    agentId: string
  ): Promise<Result<TrustRelationship, string>>
  pauseDelegationsFromMember(
    instanceId: string,
    paused: boolean
  ): Promise<Result<TrustRelationship, string>>
  /** People refused because this Nandi doesn't trust them yet (task 12) */
  listTrustRequests(): Promise<Result<TrustRequest[], string>>
  /** Trust them, or not now (`block`: refuse them from now on) */
  respondTrustRequest(
    instanceId: string,
    answer: TrustAnswer,
    block: boolean
  ): Promise<Result<null, string>>

  // Wave A: capabilities
  /** Local, registry, and network capabilities with this instance's relation */
  listCapabilities(): Promise<Result<CapabilityListing[], string>>
  getCapability(id: string): Promise<Result<CapabilityDefinition, string>>
  /** Creates or replaces `.nandi/capabilities/<id>.yaml`; returns it as saved */
  saveCapability(
    definition: CapabilityDefinition
  ): Promise<Result<CapabilityDefinition, string>>
  deleteCapability(id: string): Promise<Result<null, string>>
  transferCapabilityOwnership(
    id: string,
    newOwner: CapabilityOwner,
    reason: string | null
  ): Promise<Result<CapabilityDefinition, string>>
  /** Agents and workflows providing `id` (not `listCapabilityProviders`) */
  listProvidersForCapability(
    id: string
  ): Promise<Result<CapabilityProvider[], string>>
  /** `windowDays`: `null` is 30 */
  getCapabilityEvidence(
    id: string,
    windowDays: number | null
  ): Promise<Result<CapabilityEvidence[], string>>
  getCapabilityProviderBreakdown(
    id: string
  ): Promise<Result<CapabilityProviderStats[], string>>
  listCapabilityAmbiguities(
    id: string
  ): Promise<Result<AmbiguityCluster[], string>>
  promoteAmbiguityToSchema(
    id: string,
    clusterId: string
  ): Promise<Result<CapabilityDefinition, string>>

  // Wave A: the unified tool catalog
  listTools(): Promise<Result<ToolEntry[], string>>
  getTool(toolId: string): Promise<Result<ToolEntry, string>>
  setToolEnabled(
    toolId: string,
    enabled: boolean
  ): Promise<Result<ToolEntry, string>>
  getToolUsageStats(): Promise<Result<ToolUsageStats[], string>>

  // Wave A: capability registries (not the marketplace registries)
  listCapabilityRegistries(): Promise<Result<CapabilityRegistry[], string>>
  /** `kind`: `null` detects it from the URL */
  addCapabilityRegistry(
    url: string,
    kind: RegistryKind | null,
    branchPrefix: string | null,
    userSlug: string | null
  ): Promise<Result<CapabilityRegistry, string>>
  removeCapabilityRegistry(url: string): Promise<Result<null, string>>
  syncCapabilityRegistry(url: string): Promise<Result<SyncResult, string>>

  // Wave A: capability presence on the relay
  /** `*` for every capability */
  queryCapabilityPresence(
    capabilityId: string
  ): Promise<Result<InstancePresence[], string>>

  // Wave B: harness adapters
  listHarnesses(): Promise<Result<HarnessInfo[], string>>
  /** Probes PATH, `--version`, and the CLIs' config folders again */
  detectHarnesses(): Promise<Result<HarnessInfo[], string>>
  getHarnessHealth(harnessId: string): Promise<Result<HarnessHealth, string>>
  /** Starts the CLI over ACP, opens a session, and closes it */
  testHarness(harnessId: string): Promise<Result<HarnessTestResult, string>>
  setHarnessDefault(
    harnessId: string,
    model: string | null
  ): Promise<Result<HarnessDefault, string>>
  clearHarnessDefault(): Promise<Result<null, string>>
  getHarnessDefault(): Promise<Result<HarnessDefault | null, string>>

  /** Newest first; `limit`: `null` is 100 */
  listHarnessSessions(
    agentId: string | null,
    limit: number | null
  ): Promise<Result<HarnessSessionSummary[], string>>
  getHarnessSession(
    sessionId: string
  ): Promise<Result<HarnessSessionSummary, string>>
  getHarnessEvents(
    sessionId: string,
    afterSeq: number | null
  ): Promise<Result<HarnessEventRecord[], string>>
  killHarnessSession(sessionId: string): Promise<Result<null, string>>

  // Wave B: AWS auth, read from the user's own AWS config (never key material)
  detectAwsCli(): Promise<Result<AwsCliInfo, string>>
  listAwsProfiles(): Promise<Result<AwsProfile[], string>>
  getAwsProfile(name: string): Promise<Result<AwsProfile, string>>
  /** `aws sso login` (or `aws login` for a console profile): the AWS CLI
   * opens the browser; poll for the outcome */
  startAwsSsoLogin(profile: string): Promise<Result<SsoLoginResult, string>>
  pollAwsSsoLogin(loginId: string): Promise<Result<SsoLoginResult, string>>
  /** A new access-key profile, or new keys for one (`aws configure`) */
  saveAwsKeyProfile(input: KeyProfileInput): Promise<Result<AwsProfile, string>>
  /** The IAM Identity Center sign-in for a new SSO profile */
  startAwsSsoSetup(input: SsoSetupInput): Promise<Result<SsoSetup, string>>
  pollAwsSsoSetup(setupId: string): Promise<Result<SsoSetup, string>>
  finishAwsSsoSetup(finish: SsoSetupFinish): Promise<Result<AwsProfile, string>>
  cancelAwsSsoSetup(setupId: string): Promise<Result<null, string>>
  checkAwsCredentials(
    profile: string | null,
    region: string | null
  ): Promise<Result<CredentialStatus, string>>
  getAwsBedrockApiKeyStatus(): Promise<Result<BedrockApiKeyStatus, string>>

  // Wave B: Bedrock models and the LiteLLM gateway
  listBedrockModels(
    region: string | null
  ): Promise<Result<BedrockModel[], string>>
  refreshBedrockModels(
    region: string | null
  ): Promise<Result<BedrockModel[], string>>
  startLitellmGateway(
    model: string
  ): Promise<Result<LitellmGatewayStatus, string>>
  stopLitellmGateway(): Promise<Result<LitellmGatewayStatus, string>>
  getLitellmGatewayStatus(): Promise<Result<LitellmGatewayStatus, string>>

  // Wave CD: the ask box (see docs/developer/interaction-layer.md)
  /** Classifies and answers: inline answer, plan, edit preview, or a question back */
  ask(request: AskRequest): Promise<Result<AskResponse, string>>
  /** Returns the request ID; `ask://streaming_delta` / `ask://completed` follow */
  askStreaming(request: AskRequest): Promise<Result<string, string>>
  /** Newest first; `limit` `null` is 20 */
  listAskHistory(
    limit: number | null
  ): Promise<Result<AskHistoryEntry[], string>>

  // Wave CD: Studio's artifact editor
  loadArtifact(artifact: ArtifactRef): Promise<Result<ArtifactLoad, string>>
  /** What's open in Studio's editor (`null`: nothing), for Studio's chat */
  setStudioOpenArtifact(
    artifact: ArtifactRef | null
  ): Promise<Result<null, string>>
  saveArtifact(save: ArtifactSave): Promise<Result<ArtifactSaveResult, string>>
  /** Nothing is written */
  previewEdit(
    artifact: ArtifactRef,
    instruction: string
  ): Promise<Result<EditPreview, string>>
  listPushTargets(): Promise<Result<PushTargetOption[], string>>
  /** Would `content` save as `artifact`? Plus warnings (unknown tools, …) */
  validateArtifact(
    artifact: ArtifactRef,
    content: string
  ): Promise<Result<ArtifactValidation, string>>

  // Wave CD: work in flight
  listWork(stateFilter: WorkState | null): Promise<Result<WorkItem[], string>>
  getWorkItem(id: string): Promise<Result<WorkItem, string>>
  pauseWork(id: string): Promise<Result<null, string>>
  resumeWork(id: string): Promise<Result<null, string>>
  cancelWork(id: string): Promise<Result<null, string>>
  /** 1–5, finished delegated tasks */
  rateWork(id: string, rating: number): Promise<Result<null, string>>
  /** Hides a failed item from the Inbox (it stays in history) */
  dismissWork(id: string): Promise<Result<null, string>>
  /** Requests that came back while nobody watched, not yet dismissed */
  listRequestReturns(): Promise<Result<RequestReturn[], string>>
  dismissRequestReturn(taskId: string): Promise<Result<null, string>>
  listDismissedWork(): Promise<Result<string[], string>>

  // Wave EF: supervisor reassignment
  /** Hands a task we're serving to a teammate; returns our row */
  reassignTask(request: ReassignRequest): Promise<Result<TaskRecord, string>>
  listReassignCandidates(
    taskId: string
  ): Promise<Result<ReassignCandidate[], string>>
  // Wave EF: manual workflow steps
  completeManualStep(
    runId: string,
    stepId: string,
    completion: ManualStepCompletion
  ): Promise<Result<null, string>>
  listPendingManualSteps(): Promise<Result<PendingManualStep[], string>>
  // Wave EF: `.nandi/llm.yaml`
  getRepoLlmConfig(): Promise<Result<RepoLlmConfigStatus, string>>
  reloadRepoLlmConfig(): Promise<Result<RepoLlmConfigStatus, string>>

  /** Opens `url` in the system browser */
  openExternal(url: string): Promise<void>

  /**
   * Receives every core event. Call once at boot. The web client also
   * emits `nandi://resync` after reconnecting.
   */
  subscribe(handler: (event: NandiEvent) => void): Subscription
}
