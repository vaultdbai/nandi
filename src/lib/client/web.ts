/**
 * `NandiClient` over HTTP + WebSocket, for browsers. The page is served by
 * the Nandi web server, so requests are same-origin and authenticated by its
 * HttpOnly cookie (set when the page was opened with `?token=`).
 */

import type { Result } from '@/lib/bindings'
import type { NandiEvent } from './events'
import { createEventBus } from './event-bus'
import { marketplaceQuery } from './marketplace-filter'
import type { NandiClient } from './types'
import { withCommands } from './with-commands'

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

/**
 * One WebSocket for the page, shared by every subscriber. It reconnects
 * with backoff and asks for a resync, as events sent meanwhile are lost
 */
const socketEvents = createEventBus(emit => {
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
      if (connectedBefore) emit({ event: 'nandi://resync', payload: {} })
      connectedBefore = true
      markReady()
    }
    socket.onmessage = message => {
      try {
        emit(JSON.parse(String(message.data)) as NandiEvent)
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
})

export const webClient: NandiClient = withCommands({
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
  answerHandOver: (approvalId, answer) =>
    request('POST', `/approvals/${id(approvalId)}/hand-over`, answer),
  listPendingApprovals: () => request('GET', '/approvals/pending'),
  cancelTurn: (agentId, turnId) =>
    request('POST', `/agents/${id(agentId)}/cancel`, { turnId }),

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
  getRunPlan: runId => request('GET', `/runs/${id(runId)}/plan`),
  getRunScreenshot: (runId, path) =>
    request('POST', `/runs/${id(runId)}/screenshot`, { path }),
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

  getLlmKeyStatus: () => request('GET', '/llm-keys'),
  getDefaultModel: () => request('GET', '/default-model'),
  setDefaultModel: model => request('PUT', '/default-model', { model }),
  setFreeTrial: enabled => request('PUT', '/free-trial', { enabled }),
  getFreeTrialAccount: () => request('GET', '/free-trial/account'),
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
  agentPlaybooks: agentId =>
    request('GET', `/repo/agents/${id(agentId)}/playbooks`),
  saveAgentPlaybook: (agentId, input, previous) =>
    request('POST', `/repo/agents/${id(agentId)}/playbooks`, {
      input,
      previous,
    }),
  deleteAgentPlaybook: (agentId, name) =>
    request('DELETE', `/repo/agents/${id(agentId)}/playbooks/${id(name)}`),
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
  addPilotSecret: (agentId, pilotId, name, value) =>
    request('POST', `/agents/${id(agentId)}/pilots/${id(pilotId)}/secrets`, {
      name,
      value,
    }),
  clearBrowserProfile: (agentId, pilotId) =>
    request('DELETE', `/agents/${id(agentId)}/pilots/${id(pilotId)}/profile`),

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

  getRelayStatus: () => request('GET', '/relay/status'),
  setRelayUrl: url => request('PUT', '/relay/url', { url }),
  joinRelay: (url, joinCode) =>
    request('POST', '/relay/join', { url, joinCode }),
  getRelayInvite: () => request('GET', '/relay/invite'),
  getPublicRelayOffer: () => request('GET', '/relay/public'),
  joinPublicRelay: () => request('POST', '/relay/public/join'),
  getPublicRelayAccount: () => request('GET', '/relay/public/account'),
  listOnlineAgents: capability =>
    request('GET', `/relay/agents${query({ capability })}`),
  testRelayConnection: url => request('POST', '/relay/test', { url }),
  getRelayInstances: () => request('GET', '/relay/instances'),

  // The route still names the field `githubOrg`; it carries the registry URL

  getCompanionSnapshot: dayStart =>
    request('GET', `/companion/snapshot${query({ dayStart })}`),
  toggleCompanionWindow: () =>
    Promise.resolve({ status: 'error', error: NOT_SUPPORTED }),
  // The server keeps running; a browser tab has nothing to quit
  quitNandi: () => Promise.resolve(),
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

  getGatewayStatus: () => request('GET', '/gateway/status'),
  makeGatewayReachable: () => request('POST', '/gateway/reachable'),
  listGatewayHolds: () => request('GET', '/gateway/holds'),
  decideGatewayHold: (taskId, approve, note) =>
    request('POST', `/gateway/holds/${encodeURIComponent(taskId)}/decide`, {
      approve,
      note,
    }),
  listGatewayAssignments: () => request('GET', '/gateway/assignments'),
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

  // The relay connection lives in the core; no browser route for this yet

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

  openExternal: url => {
    window.open(url, '_blank', 'noopener,noreferrer')
    return Promise.resolve()
  },

  subscribe: handler => socketEvents.subscribe(handler),
})
