/**
 * `NandiClient` over HTTP + WebSocket, for browsers. The page is served by
 * the Nandi web server, so requests are same-origin and authenticated by its
 * HttpOnly cookie (set when the page was opened with `?token=`).
 */

import type { AccountKind, Result } from '@/lib/bindings'
import type { NandiEvent } from './events'
import { marketplaceQuery } from './marketplace-filter'
import type { NandiClient } from './types'

const NOT_SUPPORTED = 'Not supported in the browser. Use the Nandi desktop app.'
const RECONNECT_MIN_MS = 1_000
const RECONNECT_MAX_MS = 30_000
/** Boot proceeds after this even if the socket can't connect yet; the
 * reconnect then triggers a resync */
const READY_TIMEOUT_MS = 5_000

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE'

/** Calls the REST API; errors become `{ status: 'error' }` like IPC. */
async function request<T, E = string>(
  method: Method,
  path: string,
  body?: unknown
): Promise<Result<T, E>> {
  try {
    const response = await fetch(`/api${path}`, {
      method,
      credentials: 'same-origin',
      headers:
        body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    const json: unknown = await response.json().catch(() => null)
    if (response.ok) return { status: 'ok', data: json as T }
    const error =
      (json as { error?: E } | null)?.error ??
      (`Request failed (${response.status})` as E)
    return { status: 'error', error }
  } catch (error) {
    return {
      status: 'error',
      error: `Can't reach Nandi: ${String(error)}` as E,
    }
  }
}

const id = encodeURIComponent

/** Workspace routes use the connector id, the commands the `AccountKind` */
const WORKSPACE_ROUTE_IDS: Record<AccountKind, string> = {
  outlookCalendar: 'outlook-calendar',
  googleCalendar: 'google-calendar',
  oneDrive: 'onedrive',
  googleDrive: 'google-drive',
}

/** `?a=1&b=x` from the set values (`null`/`undefined` are left out) */
function query(
  params: Record<string, string | number | null | undefined>
): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value !== null && value !== undefined) search.set(key, String(value))
  }
  const encoded = search.toString()
  return encoded ? `?${encoded}` : ''
}

export const webClient: NandiClient = {
  transport: 'web',
  supports: { attachments: false },

  listAgents: () => request('GET', '/agents'),
  listAgentOverviews: todayStart =>
    request('GET', `/agents/overviews${query({ todayStart })}`),
  createAgent: input => request('POST', '/agents', input),
  startAgent: async agentId => {
    const result = await request<{ containerId: string }>(
      'POST',
      `/agents/${id(agentId)}/start`
    )
    return result.status === 'ok'
      ? { status: 'ok', data: result.data.containerId }
      : result
  },
  stopAgent: agentId => request('POST', `/agents/${id(agentId)}/stop`),
  restartAgent: async agentId => {
    const result = await request<{ containerId: string }>(
      'POST',
      `/agents/${id(agentId)}/restart`
    )
    return result.status === 'ok'
      ? { status: 'ok', data: result.data.containerId }
      : result
  },
  deleteAgent: agentId => request('DELETE', `/agents/${id(agentId)}`),

  sendMessage: (agentId, text, attachments, clientMessageId) =>
    request('POST', `/agents/${id(agentId)}/messages`, {
      text,
      attachments,
      clientMessageId,
    }),
  sendStudioMessage: (conversationId, text, attachments, clientMessageId) =>
    request('POST', `/studio/conversations/${id(conversationId)}/messages`, {
      text,
      attachments,
      clientMessageId,
    }),
  createStudioConversation: () => request('POST', '/studio/conversations'),
  approveAction: (agentId, approvalId, decision) =>
    request('POST', `/agents/${id(agentId)}/approve`, { approvalId, decision }),
  listPendingApprovals: () => request('GET', '/approvals/pending'),
  cancelTurn: (agentId, turnId) =>
    request('POST', `/agents/${id(agentId)}/cancel`, { turnId }),

  getAgentQueue: agentId => request('GET', `/agents/${id(agentId)}/queue`),
  getCurrentTurn: agentId => request('GET', `/agents/${id(agentId)}/current`),
  cancelQueuedRun: runId => request('POST', `/queue/${id(runId)}/cancel`),
  reprioritizeQueuedRun: (runId, newPriority) =>
    request('POST', `/queue/${id(runId)}/priority`, { priority: newPriority }),

  // The native picker would open on the host, not in this browser
  pickAttachments: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  discardAttachment: attachmentId =>
    request('DELETE', `/attachments/${id(attachmentId)}`),

  getAgentSnapshot: agentId =>
    request('GET', `/snapshots/agent/${id(agentId)}`),
  getDashboardSnapshot: dayStart =>
    request('GET', `/snapshots/dashboard?dayStart=${dayStart}`),
  getRunDetail: runId => request('GET', `/runs/${id(runId)}`),
  resolveRunFile: (runId, path) =>
    request('POST', `/runs/${id(runId)}/file`, { path }),
  // It would open on the host, not in this browser
  openRunFile: () => Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  getWorkflows: () => request('GET', '/workflows'),
  getWorkflow: workflowId => request('GET', `/workflows/${id(workflowId)}`),
  createWorkflow: input => request('POST', '/workflows', input),
  runWorkflow: workflowId =>
    request('POST', `/workflows/${id(workflowId)}/run`),
  listWorkflowTemplates: () => request('GET', '/workflow-templates'),
  createWorkflowFromTemplate: input =>
    request('POST', '/workflow-templates', input),
  approveWorkflowStep: (runId, stepId, decision) =>
    request('POST', `/workflow-runs/${id(runId)}/approve`, {
      stepId,
      decision,
    }),
  getStudioConversations: () => request('GET', '/studio/conversations'),
  getConversation: conversationId =>
    request('GET', `/conversations/${id(conversationId)}`),

  loadPreferences: () => request('GET', '/preferences'),
  savePreferences: preferences => request('PUT', '/preferences', preferences),
  getLlmKeyStatus: () => request('GET', '/llm-keys'),
  getDefaultModel: () => request('GET', '/default-model'),
  setDefaultModel: model => request('PUT', '/default-model', { model }),
  setFreeTrial: enabled => request('PUT', '/free-trial', { enabled }),
  testLlmKey: (provider, apiKey) =>
    request('POST', `/llm-keys/${provider}/test`, { apiKey }),
  listProviderModels: provider =>
    request('GET', `/llm-keys/${provider}/models`),
  detectOllama: () => request('GET', '/ollama'),
  setLlmApiKey: (provider, apiKey) =>
    request('PUT', `/llm-keys/${id(provider)}`, { apiKey }),
  saveEmergencyData: (filename, data) =>
    request('PUT', `/emergency-data/${id(filename)}`, data),
  loadEmergencyData: filename =>
    request('GET', `/emergency-data/${id(filename)}`),
  cleanupOldRecoveryFiles: () => request('POST', '/emergency-data/cleanup'),
  getEgressStatus: () => request('GET', '/egress/status'),
  setAwsCredentials: (accessKey, secret) =>
    request('PUT', '/egress/aws-credentials', { accessKey, secret }),
  setAwsRegion: region => request('PUT', '/egress/aws-region', { region }),

  listTriggers: () => request('GET', '/triggers'),
  getTrigger: triggerId => request('GET', `/triggers/${id(triggerId)}`),
  createTrigger: input => request('POST', '/triggers', input),
  updateTrigger: (triggerId, input) =>
    request('PUT', `/triggers/${id(triggerId)}`, input),
  deleteTrigger: triggerId => request('DELETE', `/triggers/${id(triggerId)}`),
  setTriggerEnabled: (triggerId, enabled) =>
    request(
      'POST',
      `/triggers/${id(triggerId)}/${enabled ? 'enable' : 'disable'}`
    ),
  rotateTriggerSecret: triggerId =>
    request('POST', `/triggers/${id(triggerId)}/secret`),
  testFireTrigger: (triggerId, payload) =>
    request(
      'POST',
      `/triggers/${id(triggerId)}/test`,
      payload ?? { test: true }
    ),
  listTriggerFires: (triggerId, limit) =>
    request(
      'GET',
      `/triggers/${id(triggerId)}/fires${limit === null ? '' : `?limit=${limit}`}`
    ),
  // The page is served by the Nandi web server itself
  getWebOrigin: () => Promise.resolve(location.origin),
  previewCron: schedule =>
    request('POST', '/triggers/cron-preview', { schedule }),
  previewCalendar: config =>
    request('POST', '/triggers/calendar-preview', config),

  listAvailablePacks: () => request('GET', '/marketplace/packs'),
  listMarketplacePacks: filter =>
    request('GET', `/marketplace/packs${marketplaceQuery(filter)}`),
  listAvailableAgents: filter =>
    request('GET', `/marketplace/agents${marketplaceQuery(filter)}`),
  getAgentDetail: agentId =>
    request('GET', `/marketplace/agents/${id(agentId)}/detail`),
  getPackDetail: packId =>
    request('GET', `/marketplace/packs/${id(packId)}/detail`),
  installPack: packId =>
    request('POST', `/marketplace/install/pack/${id(packId)}`),
  installAgent: agentId =>
    request('POST', `/marketplace/install/agent/${id(agentId)}`),
  inspectLocalPack: path =>
    request('POST', '/marketplace/local/inspect', { path }),
  installLocalPack: path =>
    request('POST', '/marketplace/local/install', { path }),
  listInstalled: () => request('GET', '/marketplace/installed'),
  uninstallInstalled: (kind, installedId) =>
    request('DELETE', `/marketplace/installed/${id(kind)}/${id(installedId)}`),
  forkInstalledAgent: (installedId, newId, newName) =>
    request('POST', `/marketplace/fork/${id(installedId)}`, {
      newId,
      newName,
    }),
  checkForUpdates: () => request('GET', '/marketplace/updates'),
  applyUpdate: (installedId, newRef) =>
    request('POST', `/marketplace/updates/${id(installedId)}/apply`, {
      newRef,
    }),
  getAgentConsent: agentId =>
    request('GET', `/marketplace/consent/${id(agentId)}`),
  grantAgentConsent: (agentId, categories) =>
    request('POST', `/marketplace/consent/${id(agentId)}`, { categories }),
  listRegistries: () => request('GET', '/marketplace/registries'),
  addRegistry: input => request('POST', '/marketplace/registries', input),
  removeRegistry: url =>
    request('DELETE', `/marketplace/registries/${id(url)}`),
  createCommunityRegistry: () =>
    request('POST', '/marketplace/community/create'),
  publishAgent: input => request('POST', '/marketplace/publish', input),

  // Agents repo, pilots, run history, audit, connectors, and the network
  saveAgent: (agentId, input) =>
    request('POST', `/repo/agents/${id(agentId)}/save`, input),
  saveAgentInstructions: (agentId, instructions) =>
    request('POST', `/repo/agents/${id(agentId)}/instructions`, {
      instructions,
    }),
  resetAgentInstructions: agentId =>
    request('DELETE', `/repo/agents/${id(agentId)}/instructions`),
  loadEntity: (kind, entityId) =>
    request('GET', `/repo/entities/${id(kind)}/${id(entityId)}`),
  repoStatus: () => request('GET', '/repo/status'),
  gitStatus: () => request('GET', '/repo/git/status'),
  gitDiff: path => request('GET', `/repo/git/diff${query({ path })}`),
  gitCommit: message => request('POST', '/repo/git/commit', { message }),

  listPilots: () => request('GET', '/pilots'),
  installPilot: pilotId => request('POST', `/pilots/${id(pilotId)}/install`),
  startPilot: pilotId => request('POST', `/pilots/${id(pilotId)}/start`),
  stopPilot: pilotId => request('POST', `/pilots/${id(pilotId)}/stop`),
  pilotStatus: pilotId => request('GET', `/pilots/${id(pilotId)}/status`),
  grantPilotCapability: (agentId, pilotId, capability) =>
    request('POST', `/agents/${id(agentId)}/pilots/${id(pilotId)}/grant`, {
      capability,
    }),
  revokePilotCapability: (agentId, pilotId, capability) =>
    request('POST', `/agents/${id(agentId)}/pilots/${id(pilotId)}/revoke`, {
      capability,
    }),
  agentPilotPermissions: agentId =>
    request('GET', `/agents/${id(agentId)}/pilots`),
  setPilotSecret: (agentId, pilotId, name, value) =>
    request('PUT', `/agents/${id(agentId)}/pilots/${id(pilotId)}/secret`, {
      name,
      value,
    }),
  agentPolicies: agentId => request('GET', `/agents/${id(agentId)}/policies`),
  listPolicies: () => request('GET', '/policies'),

  listAllRuns: (filters, limit, offset) =>
    request('GET', `/runs${query({ ...filters, limit, offset })}`),
  getRunStats: range => request('GET', `/runs/stats${query({ ...range })}`),
  listAuditLog: (kind, limit, offset) =>
    request('GET', `/audit/${id(kind)}${query({ limit, offset })}`),

  listConnectors: () => request('GET', '/connectors'),
  installConnector: connectorId =>
    request('POST', `/connectors/${id(connectorId)}/install`),
  startConnector: connectorId =>
    request('POST', `/connectors/${id(connectorId)}/start`),
  stopConnector: connectorId =>
    request('POST', `/connectors/${id(connectorId)}/stop`),
  configureConnector: (connectorId, config) =>
    request('PUT', `/connectors/${id(connectorId)}/config`, { config }),
  getAgentConnectors: agentId =>
    request('GET', `/agents/${id(agentId)}/connectors`),
  setAgentConnectors: (agentId, connectors) =>
    request('PUT', `/agents/${id(agentId)}/connectors`, { connectors }),
  setConnectorSecret: (connectorId, key, value) =>
    request('PUT', `/connectors/${id(connectorId)}/secret`, { key, value }),
  connectorStatus: connectorId =>
    request('GET', `/connectors/${id(connectorId)}/status`),

  findAgentsByCapability: capability =>
    request('GET', `/capabilities/${id(capability)}/agents`),
  listDeclaredCapabilities: () => request('GET', '/capabilities/declared'),
  getAgentCard: agentId => request('GET', `/agents/${id(agentId)}/card`),

  getRelayStatus: () => request('GET', '/relay/status'),
  setRelayUrl: url => request('PUT', '/relay/url', { url }),
  joinRelay: (url, joinCode) =>
    request('POST', '/relay/join', { url, joinCode }),
  getRelayInvite: () => request('GET', '/relay/invite'),
  listOnlineAgents: capability =>
    request('GET', `/relay/agents${query({ capability })}`),
  testRelayConnection: url => request('POST', '/relay/test', { url }),
  getRelayInstances: () => request('GET', '/relay/instances'),

  listAvailableProviders: capability =>
    request('GET', `/delegation/providers${query({ capability })}`),
  delegateTask: (capability, input, target) =>
    request('POST', '/delegation/tasks', { capability, input, target }),
  listDelegationHistory: limit =>
    request('GET', `/delegation/history${query({ limit })}`),
  getAgentDelegation: agentId =>
    request('GET', `/agents/${id(agentId)}/delegation`),
  setAgentDelegation: (agentId, settings) =>
    request('PUT', `/agents/${id(agentId)}/delegation`, settings),

  listTeams: () => request('GET', '/teams'),
  getTeam: teamId => request('GET', `/teams/${id(teamId)}`),
  // The route still names the field `githubOrg`; it carries the registry URL
  createTeam: registryUrl =>
    request('POST', '/teams', { githubOrg: registryUrl }),
  joinTeam: teamId => request('POST', `/teams/${id(teamId)}/join`),
  leaveTeam: teamId => request('POST', `/teams/${id(teamId)}/leave`),
  syncTeamRegistry: teamId => request('POST', `/teams/${id(teamId)}/sync`),
  shareAgent: (teamId, agentId) =>
    request('POST', `/teams/${id(teamId)}/agents/${id(agentId)}`),
  unshareAgent: (teamId, agentId) =>
    request('DELETE', `/teams/${id(teamId)}/agents/${id(agentId)}`),

  listAgentsSharedWithMe: () => request('GET', '/shared/with-me'),
  listAgentsSharedByMe: () => request('GET', '/shared/by-me'),
  listCapabilityProviders: capability =>
    request(
      'GET',
      capability === null
        ? '/shared/capabilities'
        : `/shared/capabilities?capability=${encodeURIComponent(capability)}`
    ),
  setProviderPin: (capability, cardId) =>
    request('PUT', '/shared/pins', { capability, cardId }),
  listTrustPolicies: () => request('GET', '/trust/policies'),
  setTrustPolicy: (requester, providerAgent, policy) =>
    request('PUT', '/trust/policies', { requester, providerAgent, policy }),

  analyzeRun: (runId, question) =>
    request('POST', `/runs/${id(runId)}/analyze`, { question }),
  summarizeRun: runId => request('GET', `/runs/${id(runId)}/summary`),

  getWorkflowGraph: workflowId =>
    request('GET', `/workflows/${id(workflowId)}/graph`),
  saveWorkflowYaml: (workflowId, yaml) =>
    request('PUT', `/workflows/${id(workflowId)}/yaml`, { yaml }),

  searchMarketplace: q => request('GET', `/marketplace/search${query({ q })}`),

  getCompanionSnapshot: dayStart =>
    request('GET', `/companion/snapshot${query({ dayStart })}`),
  toggleCompanionWindow: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  // The server keeps running; a browser tab has nothing to quit
  quitNandi: () => Promise.resolve(),
  getImageStatus: image => request('GET', `/images/${image}`),
  buildImage: image => request('POST', `/images/${image}/build`),
  pullImage: image => request('POST', `/images/${image}/pull`),
  getToolsStatus: bundle => request('GET', `/tool-bundles/${bundle}`),
  installTools: bundle => request('POST', `/tool-bundles/${bundle}/install`),
  // No companion window in a browser
  placeCompanion: () => Promise.resolve({ status: 'ok', data: null }),
  setCompanionBubble: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  setCompanionStartBisonOnly: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  listFolderUsers: connectorId =>
    request('GET', `/connectors/${id(connectorId)}/folder-users`),
  setCompanionPinned: pinned => request('POST', '/companion/pin', { pinned }),
  sendCompanionMessage: (text, conversationId, scopeRequestId) =>
    request('POST', '/companion/messages', {
      text,
      conversationId,
      scopeRequestId,
    }),
  setCompanionNotify: notify =>
    request('POST', '/companion/notify', { notify }),
  setCompanionNotifyResults: notify =>
    request('POST', '/companion/notify-results', { notify }),
  // The web build's companion lives in the page (`watchInCompanion`)
  companionFocusRequest: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),

  searchPalette: q => request('GET', `/palette/search${query({ q })}`),
  executePaletteAction: (actionId, args) =>
    request('POST', `/palette/actions/${id(actionId)}`, { args }),

  loadAgentMemory: agentId => request('GET', `/agents/${id(agentId)}/memory`),
  updateAgentMemory: (agentId, index, content) =>
    request('PUT', `/agents/${id(agentId)}/memory/${index}`, { content }),
  deleteAgentMemory: (agentId, index) =>
    request('DELETE', `/agents/${id(agentId)}/memory/${index}`),
  refreshAgentMemory: agentId =>
    request('POST', `/agents/${id(agentId)}/memory/refresh`),
  getAgentMemoryEnabled: agentId =>
    request('GET', `/agents/${id(agentId)}/memory-enabled`),
  setAgentMemoryEnabled: (agentId, enabled) =>
    request('PUT', `/agents/${id(agentId)}/memory-enabled`, { enabled }),

  getIdentityStatus: () => request('GET', '/identity/status'),
  setDisplayName: name => request('PUT', '/identity/display-name', { name }),
  setUserSlug: slug => request('PUT', '/identity/user-slug', { slug }),
  setOnboardingStep: step =>
    request('PUT', '/identity/onboarding-step', { step }),

  getGithubSignIn: () => request('GET', '/github/sign-in'),
  startGithubSignIn: () => request('POST', '/github/sign-in/start'),
  pollGithubSignIn: () => request('POST', '/github/sign-in/poll'),
  signOutGithub: () => request('DELETE', '/github/sign-in'),

  getRemoteStatus: () => request('GET', '/remote/status'),
  configureRemote: (url, credential, prefix, slug) =>
    request('PUT', '/remote/config', { url, credential, prefix, slug }),
  clearRemote: () => request('DELETE', '/remote'),
  testRemoteConnection: () => request('POST', '/remote/test'),
  pushToRemote: overwrite => request('POST', '/remote/push', { overwrite }),
  pullFromRemote: () => request('POST', '/remote/pull'),
  listRemoteBranches: () => request('GET', '/remote/branches'),

  getTask: taskId => request('GET', `/tasks/${id(taskId)}`),
  listTasks: filter => request('GET', `/tasks${query({ ...filter })}`),
  cancelTask: (taskId, reason) =>
    request('POST', `/tasks/${id(taskId)}/cancel`, { reason }),
  queryTaskStatus: taskId => request('GET', `/tasks/${id(taskId)}/status`),
  getTaskEvents: taskId => request('GET', `/tasks/${id(taskId)}/events`),
  getTaskMessages: taskId => request('GET', `/tasks/${id(taskId)}/messages`),
  getEtaEstimate: (capability, providerAgent) =>
    request('GET', `/eta/${id(capability)}${query({ providerAgent })}`),
  getRequest: requestId => request('GET', `/requests/${id(requestId)}`),
  requestForRun: runId => request('GET', `/runs/${id(runId)}/request`),
  retryTask: requestId => request('POST', `/requests/${id(requestId)}/retry`),
  startRequest: (plan, text) => request('POST', '/requests', { plan, text }),
  completeRequestStep: (requestId, position) =>
    request('POST', `/requests/${id(requestId)}/steps/${position}/complete`),
  cancelRequest: requestId =>
    request('POST', `/requests/${id(requestId)}/cancel`),
  mascotGet: () => request('GET', '/mascot'),
  mascotSet: name => request('PUT', '/mascot', { name }),
  mascotPhrase: input => request('POST', '/mascot/phrase', { input }),
  rerunRequest: requestId =>
    request('POST', `/requests/${id(requestId)}/rerun`),
  draftWorkflowFromRequest: requestId =>
    request('POST', `/requests/${id(requestId)}/workflow-draft`),

  respondToClarification: ({ taskId, round, answers }) =>
    request('POST', `/delegation/${id(taskId)}/clarify`, { round, answers }),
  escalateClarification: (taskId, questionId) =>
    request('POST', `/delegation/${id(taskId)}/escalate/${id(questionId)}`),
  getPendingClarifications: () =>
    request('GET', '/delegation/pending-clarifications'),
  getClarificationPolicy: agentId =>
    request('GET', `/agents/${id(agentId)}/clarification-policy`),
  setClarificationPolicy: (agentId, policy) =>
    request('PUT', `/agents/${id(agentId)}/clarification-policy`, policy),

  getGatewayStatus: () => request('GET', '/gateway/status'),
  makeGatewayReachable: () => request('POST', '/gateway/reachable'),
  listGatewayHolds: () => request('GET', '/gateway/holds'),
  decideGatewayHold: (taskId, approve) =>
    request('POST', `/gateway/holds/${encodeURIComponent(taskId)}/decide`, {
      approve,
    }),
  listGatewayAssignments: () => request('GET', '/gateway/assignments'),
  getMyNotes: () => request('GET', '/notes'),
  readNote: name =>
    request('GET', `/notes/file?name=${encodeURIComponent(name)}`),
  saveNote: (name, content) =>
    request('PUT', `/notes/file?name=${encodeURIComponent(name)}`, {
      content,
    }),
  deleteNote: name =>
    request('DELETE', `/notes/file?name=${encodeURIComponent(name)}`),
  setNotesAgentsCanWrite: allow =>
    request('PUT', '/notes/agents-can-write', { allow }),
  listRecentChanges: () => request('GET', '/changes'),
  refreshFromDisk: () => request('POST', '/changes/refresh'),
  getEmailStatus: () => request('GET', '/email'),
  startEmailSignIn: provider =>
    request('POST', `/email/${encodeURIComponent(provider)}/sign-in`),
  pollEmailSignIn: provider =>
    request('POST', `/email/${encodeURIComponent(provider)}/sign-in/poll`),
  checkEmailNow: () => request('POST', '/email/check'),
  setEmailPaused: (provider, paused) =>
    request('PUT', `/email/${encodeURIComponent(provider)}/paused`, {
      paused,
    }),
  disconnectEmail: provider =>
    request('DELETE', `/email/${encodeURIComponent(provider)}`),
  setEmailSettings: settings => request('PUT', '/email/settings', settings),
  setEmailBackfill: (provider, action) =>
    request('PUT', `/email/${encodeURIComponent(provider)}/backfill`, {
      action,
    }),
  estimateEmailBackfill: range =>
    request('POST', '/email/backfill/estimate', { range }),
  listConnectorGrants: () => request('GET', '/connector-grants'),
  grantConnectorAccess: (agentId, connectorId, access, scope) =>
    request('POST', '/connector-grants', {
      agentId,
      connectorId,
      access,
      scope,
    }),
  revokeConnectorAccess: grant =>
    request('POST', '/connector-grants/revoke', grant),
  answerAccessRequest: (approvalId, choice) =>
    request('POST', `/access-requests/${id(approvalId)}`, { choice }),
  listRegistryConnectors: refresh =>
    request('GET', `/registry-connectors?refresh=${refresh}`),
  getConnectorConsent: (registryUrl, connectorId) =>
    request('POST', '/registry-connectors/consent', {
      registryUrl,
      connectorId,
    }),
  installRegistryConnector: (registryUrl, connectorId, apiKey) =>
    request('POST', '/registry-connectors/install', {
      registryUrl,
      connectorId,
      apiKey,
    }),
  removeConnector: connectorId =>
    request('POST', `/connectors/${id(connectorId)}/remove`),
  scaffoldConnector: input => request('POST', '/connectors/scaffold', input),
  publishConnector: input => request('POST', '/connectors/publish', input),
  getWorkspaceStatus: () => request('GET', '/workspace'),
  startWorkspaceSignIn: kind =>
    request('POST', `/workspace/${WORKSPACE_ROUTE_IDS[kind]}/sign-in`),
  pollWorkspaceSignIn: kind =>
    request('POST', `/workspace/${WORKSPACE_ROUTE_IDS[kind]}/sign-in/poll`),
  disconnectWorkspace: kind =>
    request('DELETE', `/workspace/${WORKSPACE_ROUTE_IDS[kind]}`),
  checkCalendarNow: () => request('POST', '/workspace/calendar/check'),
  setCalendarInterval: minutes =>
    request('PUT', '/workspace/calendar/interval', { minutes }),
  listNoteSuggestions: () => request('GET', '/notes/suggestions'),
  answerNoteSuggestion: (suggestionId, accept) =>
    request('POST', `/notes/suggestions/${id(suggestionId)}`, { accept }),
  closeGatewayAssignment: (taskId, done, note) =>
    request(
      'POST',
      `/gateway/assignments/${encodeURIComponent(taskId)}/close`,
      { done, note }
    ),
  getGatewayConfig: () => request('GET', '/gateway/config'),
  setGatewayConfig: config => request('PUT', '/gateway/config', config),
  listGatewayCallers: () => request('GET', '/gateway/callers'),
  setCallerPolicy: policy =>
    request('PUT', `/gateway/callers/${id(policy.callerInstance)}`, policy),
  removeCallerPolicy: callerInstance =>
    request('DELETE', `/gateway/callers/${id(callerInstance)}`),
  getGatewayUsage: callerInstance =>
    request('GET', `/gateway/usage/${id(callerInstance)}`),
  getGatewayManifest: () => request('GET', '/gateway/manifest'),
  setGatewayManifest: manifest => request('PUT', '/gateway/manifest', manifest),
  gatewayChat: message => request('POST', '/gateway/chat', { message }),

  getNetworkSnapshot: () => request('GET', '/network/snapshot'),
  refreshNetwork: () => request('POST', '/network/refresh'),

  getTeamRoster: teamId => request('GET', `/teams/${id(teamId)}/roster`),
  getMemberProfile: (teamId, handle) =>
    request('GET', `/teams/${id(teamId)}/members/${id(handle)}/profile`),
  getMemberByInstance: instanceId =>
    request('GET', `/members/by-instance/${id(instanceId)}`),
  acceptPeerKey: instanceId =>
    request('POST', `/members/${id(instanceId)}/key/accept`),
  setTrustRelationship: (instanceId, update) =>
    request('PUT', `/members/${id(instanceId)}/trust`, update),
  getRelayTrust: () => request('GET', '/trust/relay'),
  setRelayTrust: enabled => request('PUT', '/trust/relay', { enabled }),
  blockAgentFromMember: (instanceId, agentId) =>
    request('POST', `/members/${id(instanceId)}/agents/${id(agentId)}/block`),
  unblockAgentFromMember: (instanceId, agentId) =>
    request('DELETE', `/members/${id(instanceId)}/agents/${id(agentId)}/block`),
  pauseDelegationsFromMember: (instanceId, paused) =>
    request('POST', `/members/${id(instanceId)}/delegations/pause`, {
      paused,
    }),
  listTrustRequests: () => request('GET', '/trust/requests'),
  respondTrustRequest: (instanceId, answer, block) =>
    request('POST', `/trust/requests/${id(instanceId)}`, { answer, block }),

  listCapabilities: () => request('GET', '/capabilities'),
  getCapability: capabilityId =>
    request('GET', `/capabilities/${id(capabilityId)}`),
  saveCapability: definition =>
    request('PUT', `/capabilities/${id(definition.id)}`, definition),
  deleteCapability: capabilityId =>
    request('DELETE', `/capabilities/${id(capabilityId)}`),
  transferCapabilityOwnership: (capabilityId, newOwner, reason) =>
    request('POST', `/capabilities/${id(capabilityId)}/transfer`, {
      newOwner,
      reason,
    }),
  listProvidersForCapability: capabilityId =>
    request('GET', `/capabilities/${id(capabilityId)}/providers`),
  getCapabilityEvidence: (capabilityId, windowDays) =>
    request(
      'GET',
      `/capabilities/${id(capabilityId)}/evidence${query({ windowDays })}`
    ),
  getCapabilityProviderBreakdown: capabilityId =>
    request('GET', `/capabilities/${id(capabilityId)}/breakdown`),
  listCapabilityAmbiguities: capabilityId =>
    request('GET', `/capabilities/${id(capabilityId)}/ambiguities`),
  promoteAmbiguityToSchema: (capabilityId, clusterId) =>
    request(
      'POST',
      `/capabilities/${id(capabilityId)}/ambiguities/${id(clusterId)}/promote`
    ),

  listTools: () => request('GET', '/tools'),
  getTool: toolId => request('GET', `/tools/${id(toolId)}`),
  setToolEnabled: (toolId, enabled) =>
    request('PUT', `/tools/${id(toolId)}/enabled`, { enabled }),
  getToolUsageStats: () => request('GET', '/tools/usage'),

  listCapabilityRegistries: () => request('GET', '/capability-registries'),
  addCapabilityRegistry: (url, kind, branchPrefix, userSlug) =>
    request('POST', '/capability-registries', {
      url,
      kind,
      branchPrefix,
      userSlug,
    }),
  removeCapabilityRegistry: url =>
    request('DELETE', `/capability-registries${query({ url })}`),
  syncCapabilityRegistry: url =>
    request('POST', '/capability-registries/sync', { url }),

  // The relay connection lives in the core; no browser route for this yet
  queryCapabilityPresence: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),

  listHarnesses: () => request('GET', '/harnesses'),
  detectHarnesses: () => request('POST', '/harnesses/detect'),
  getHarnessHealth: harnessId =>
    request('GET', `/harnesses/${id(harnessId)}/health`),
  testHarness: harnessId => request('POST', `/harnesses/${id(harnessId)}/test`),
  setHarnessDefault: (harnessId, model) =>
    request('PUT', '/harnesses/default', { harnessId, model }),
  clearHarnessDefault: () => request('DELETE', '/harnesses/default'),
  getHarnessDefault: () => request('GET', '/harnesses/default'),

  listHarnessSessions: (agentId, limit) =>
    request('GET', `/harness-sessions${query({ agentId, limit })}`),
  getHarnessSession: sessionId =>
    request('GET', `/harness-sessions/${id(sessionId)}`),
  getHarnessEvents: (sessionId, afterSeq) =>
    request(
      'GET',
      `/harness-sessions/${id(sessionId)}/events${query({ afterSeq })}`
    ),
  killHarnessSession: sessionId =>
    request('DELETE', `/harness-sessions/${id(sessionId)}`),

  detectAwsCli: () => request('GET', '/aws/cli'),
  listAwsProfiles: () => request('GET', '/aws/profiles'),
  getAwsProfile: name => request('GET', `/aws/profiles/${id(name)}`),
  startAwsSsoLogin: profile => request('POST', '/aws/sso/login', { profile }),
  pollAwsSsoLogin: loginId => request('GET', `/aws/sso/login/${id(loginId)}`),
  saveAwsKeyProfile: input => request('POST', '/aws/profiles/keys', input),
  startAwsSsoSetup: input => request('POST', '/aws/sso/setup', input),
  pollAwsSsoSetup: setupId => request('GET', `/aws/sso/setup/${id(setupId)}`),
  finishAwsSsoSetup: finish => request('POST', '/aws/sso/setup/finish', finish),
  cancelAwsSsoSetup: setupId =>
    request('DELETE', `/aws/sso/setup/${id(setupId)}`),
  checkAwsCredentials: (profile, region) =>
    request('POST', '/aws/credentials/check', { profile, region }),
  getAwsBedrockApiKeyStatus: () => request('GET', '/aws/bedrock-api-key'),

  listBedrockModels: region =>
    request('GET', `/bedrock/models${query({ region })}`),
  refreshBedrockModels: region =>
    request('POST', '/bedrock/models/refresh', { region }),
  startLitellmGateway: model =>
    request('POST', '/bedrock/gateway/start', { model }),
  stopLitellmGateway: () => request('POST', '/bedrock/gateway/stop'),
  getLitellmGatewayStatus: () => request('GET', '/bedrock/gateway'),

  ask: body => request('POST', '/ask', body),
  askStreaming: body => request('POST', '/ask/stream', body),
  listAskHistory: limit => request('GET', `/ask/history${query({ limit })}`),

  loadArtifact: artifact => request('POST', '/studio/load', { artifact }),
  setStudioOpenArtifact: artifact =>
    request('POST', '/studio/open', { artifact }),
  saveArtifact: save => request('POST', '/studio/save', save),
  previewEdit: (artifact, instruction) =>
    request('POST', '/studio/preview', { artifact, instruction }),
  listPushTargets: () => request('GET', '/studio/push-targets'),
  validateArtifact: (artifact, content) =>
    request('POST', '/studio/validate', { artifact, content }),

  listWork: stateFilter =>
    request('GET', `/work${query({ state: stateFilter })}`),
  getWorkItem: workId => request('GET', `/work/${id(workId)}`),
  pauseWork: workId => request('POST', `/work/${id(workId)}/pause`),
  resumeWork: workId => request('POST', `/work/${id(workId)}/resume`),
  cancelWork: workId => request('POST', `/work/${id(workId)}/cancel`),
  rateWork: (workId, rating) =>
    request('POST', `/work/${id(workId)}/rating`, { rating }),
  dismissWork: workId => request('POST', `/work/${id(workId)}/dismiss`),
  listRequestReturns: () => request('GET', '/delegation/returns'),
  dismissRequestReturn: taskId =>
    request('POST', `/delegation/returns/${id(taskId)}/dismiss`),
  listDismissedWork: () => request('GET', '/work/dismissed'),

  reassignTask: ({ taskId, newProviderInstance, reason }) =>
    request('POST', `/tasks/${id(taskId)}/reassign`, {
      newProviderInstance,
      reason,
    }),
  listReassignCandidates: taskId =>
    request('GET', `/tasks/${id(taskId)}/reassign-candidates`),
  completeManualStep: (runId, stepId, completion) =>
    request(
      'POST',
      `/workflows/${id(runId)}/steps/${id(stepId)}/complete`,
      completion
    ),
  listPendingManualSteps: () => request('GET', '/workflows/manual-steps'),
  getRepoLlmConfig: () => request('GET', '/config/llm'),
  reloadRepoLlmConfig: () => request('POST', '/config/llm/reload'),

  openExternal: url => {
    window.open(url, '_blank', 'noopener,noreferrer')
    return Promise.resolve()
  },

  subscribe(handler) {
    let socket: WebSocket | null = null
    let retryDelay = RECONNECT_MIN_MS
    let retryTimer: ReturnType<typeof setTimeout> | undefined
    let connectedBefore = false
    let closed = false
    let markReady: () => void = () => undefined
    const ready = new Promise<void>(resolve => {
      markReady = resolve
      setTimeout(resolve, READY_TIMEOUT_MS)
    })

    const connect = () => {
      const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
      socket = new WebSocket(`${protocol}//${location.host}/ws`)

      socket.onopen = () => {
        retryDelay = RECONNECT_MIN_MS
        // Anything emitted while disconnected is only in the DB now
        if (connectedBefore) handler({ event: 'nandi://resync', payload: {} })
        connectedBefore = true
        markReady()
      }
      socket.onmessage = message => {
        try {
          handler(JSON.parse(String(message.data)) as NandiEvent)
        } catch {
          // Ignore malformed frames
        }
      }
      socket.onclose = () => {
        if (closed) return
        retryTimer = setTimeout(connect, retryDelay)
        retryDelay = Math.min(retryDelay * 2, RECONNECT_MAX_MS)
      }
    }

    connect()
    return {
      ready,
      unsubscribe: () => {
        closed = true
        clearTimeout(retryTimer)
        socket?.close()
      },
    }
  },
}
