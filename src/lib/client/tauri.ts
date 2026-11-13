/** `NandiClient` over Tauri IPC: the generated tauri-specta bindings. */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { commands } from '@/lib/tauri-bindings'
import { CORE_EVENT_NAMES, type NandiEvent } from './events'
import { createEventBus } from './event-bus'
import { marketplaceFilter } from './marketplace-filter'
import type { NandiClient } from './types'
import { withCommands } from './with-commands'

/** One `listen()` per core event, shared by every subscriber */
const coreEvents = createEventBus(emit => {
  const subscriptions: Promise<UnlistenFn>[] = CORE_EVENT_NAMES.map(name =>
    listen(name, e => emit({ event: name, payload: e.payload } as NandiEvent))
  )
  return {
    ready: Promise.all(subscriptions).then(() => undefined),
    unsubscribe: () => {
      for (const subscription of subscriptions) {
        void subscription.then(unlisten => unlisten())
      }
    },
  }
})

export const tauriClient: NandiClient = withCommands({
  transport: 'tauri',
  supports: { attachments: true },

  listAgents: () => commands.listAgents(),
  listAgentOverviews: todayStart => commands.listAgentOverviews(todayStart),
  createAgent: input => commands.createAgent(input),
  startAgent: agentId => commands.startAgent(agentId),
  stopAgent: agentId => commands.stopAgent(agentId),
  restartAgent: agentId => commands.restartAgent(agentId),
  deleteAgent: agentId => commands.deleteAgent(agentId),

  sendMessage: (agentId, text, attachments, clientMessageId) =>
    commands.sendMessage(agentId, text, attachments, clientMessageId),
  sendStudioMessage: (conversationId, text, attachments, clientMessageId) =>
    commands.sendStudioMessage(
      conversationId,
      text,
      attachments,
      clientMessageId
    ),
  createStudioConversation: () => commands.createStudioConversation(),
  approveAction: (agentId, approvalId, decision) =>
    commands.approveAction(agentId, approvalId, decision),
  answerHandOver: (approvalId, answer) =>
    commands.answerHandOver(approvalId, answer),
  listPendingApprovals: () => commands.listPendingApprovals(),
  cancelTurn: (agentId, turnId) => commands.cancelTurn(agentId, turnId),

  pickAttachments: kind => commands.pickAttachments(kind),
  discardAttachment: id => commands.discardAttachment(id),

  getAgentSnapshot: agentId => commands.getAgentSnapshot(agentId),
  getDashboardSnapshot: dayStart => commands.getDashboardSnapshot(dayStart),
  getRunDetail: runId => commands.getRunDetail(runId),
  resolveRunFile: (runId, path) => commands.resolveRunFile(runId, path),
  getRunPlan: runId => commands.getRunPlan(runId),
  getRunScreenshot: (runId, path) => commands.getRunScreenshot(runId, path),
  openRunFile: (runId, path, reveal) =>
    commands.openRunFile(runId, path, reveal),
  getWorkflows: () => commands.getWorkflows(),
  getStudioConversations: () => commands.getStudioConversations(),
  getConversation: conversationId => commands.getConversation(conversationId),

  getLlmKeyStatus: () => commands.getLlmKeyStatus(),
  getDefaultModel: () => commands.getDefaultModel(),
  setDefaultModel: model => commands.setDefaultModel(model),
  setFreeTrial: enabled => commands.setFreeTrial(enabled),
  getFreeTrialAccount: () => commands.getFreeTrialAccount(),
  testLlmKey: (provider, apiKey) => commands.testLlmKey(provider, apiKey),
  listProviderModels: provider => commands.listProviderModels(provider),
  detectOllama: () => commands.detectOllama(),
  setLlmApiKey: (provider, apiKey) => commands.setLlmApiKey(provider, apiKey),
  saveEmergencyData: (filename, data) =>
    commands.saveEmergencyData(filename, data),
  loadEmergencyData: filename => commands.loadEmergencyData(filename),
  cleanupOldRecoveryFiles: () => commands.cleanupOldRecoveryFiles(),

  getWorkflow: workflowId => commands.getWorkflow(workflowId),
  createWorkflow: input => commands.createWorkflow(input),
  runWorkflow: workflowId => commands.runWorkflow(workflowId),
  listWorkflowTemplates: () => commands.listWorkflowTemplates(),
  createWorkflowFromTemplate: input =>
    commands.createWorkflowFromTemplate(input),
  approveWorkflowStep: (runId, stepId, decision) =>
    commands.approveWorkflowStep(runId, stepId, decision),

  getWebOrigin: async () => {
    const result = await commands.getWebOrigin()
    return result.status === 'ok' ? result.data : null
  },

  listAvailablePacks: () => commands.listAvailablePacks(),
  listMarketplacePacks: filter =>
    commands.listMarketplacePacks(marketplaceFilter(filter)),
  listAvailableAgents: filter =>
    commands.listAvailableAgents(marketplaceFilter(filter)),
  getAgentDetail: agentId => commands.getAgentDetail(agentId),
  getPackDetail: packId => commands.getPackDetail(packId),
  installPack: packId => commands.installPack(packId),
  inspectLocalPack: path => commands.inspectLocalPack(path),
  installLocalPack: path => commands.installLocalPack(path),
  installAgent: agentId => commands.installAgent(agentId),
  listInstalled: () => commands.listInstalled(),
  uninstallInstalled: (kind, id) => commands.uninstallInstalled(kind, id),
  forkInstalledAgent: (installedId, newId, newName) =>
    commands.forkInstalledAgent(installedId, newId, newName),
  checkForUpdates: () => commands.checkForUpdates(),
  applyUpdate: (installedId, newRef) =>
    commands.applyUpdate(installedId, newRef),
  getAgentConsent: agentId => commands.getAgentConsent(agentId),
  grantAgentConsent: (agentId, categories) =>
    commands.grantAgentConsent(agentId, categories),
  listRegistries: () => commands.listRegistries(),
  addRegistry: input => commands.addRegistry(input),
  removeRegistry: url => commands.removeRegistry(url),
  createCommunityRegistry: () => commands.createCommunityRegistry(),
  publishAgent: input => commands.publishAgent(input),

  // Agents repo, pilots, run history, audit, connectors, and the network
  saveAgent: (agentId, input) => commands.repoSaveAgent(agentId, input),
  saveAgentInstructions: (agentId, instructions) =>
    commands.repoSaveAgentInstructions(agentId, instructions),
  resetAgentInstructions: agentId =>
    commands.repoResetAgentInstructions(agentId),
  agentPlaybooks: agentId => commands.repoAgentPlaybooks(agentId),
  saveAgentPlaybook: (agentId, input, previous) =>
    commands.repoSaveAgentPlaybook(agentId, input, previous),
  deleteAgentPlaybook: (agentId, name) =>
    commands.repoDeleteAgentPlaybook(agentId, name),
  loadEntity: (kind, id) => commands.repoLoadEntity(kind, id),
  repoStatus: () => commands.repoStatus(),
  gitStatus: () => commands.repoGitStatus(),
  gitDiff: path => commands.repoGitDiff(path),
  gitCommit: message => commands.repoGitCommit(message),

  listConnectors: () => commands.listConnectors(),
  installConnector: id => commands.installConnector(id),
  startConnector: id => commands.startConnector(id),
  stopConnector: id => commands.stopConnector(id),
  configureConnector: (id, config) => commands.configureConnector(id, config),
  getAgentConnectors: agentId => commands.getAgentConnectors(agentId),
  setAgentConnectors: (agentId, connectors) =>
    commands.setAgentConnectors(agentId, connectors),
  setConnectorSecret: (id, key, value) =>
    commands.setConnectorSecret(id, key, value),
  connectorStatus: id => commands.connectorStatus(id),

  getRelayStatus: () => commands.getRelayStatus(),
  setRelayUrl: url => commands.setRelayUrl(url),
  joinRelay: (url, joinCode) => commands.joinRelay(url, joinCode),
  getRelayInvite: () => commands.getRelayInvite(),
  getPublicRelayOffer: () => commands.getPublicRelayOffer(),
  joinPublicRelay: () => commands.joinPublicRelay(),
  getPublicRelayAccount: () => commands.getPublicRelayAccount(),
  listOnlineAgents: capability => commands.listOnlineAgents(capability),
  testRelayConnection: url => commands.testRelayConnection(url),
  getRelayInstances: () => commands.getRelayInstances(),

  getCompanionSnapshot: dayStart => commands.getCompanionSnapshot(dayStart),
  toggleCompanionWindow: () => commands.toggleCompanionWindow(),
  quitNandi: () => commands.quitNandi(),
  placeCompanion: shape => commands.placeCompanion(shape),
  setCompanionBubble: bubble => commands.setCompanionBubble(bubble),
  setCompanionStartBisonOnly: on => commands.setCompanionStartBisonOnly(on),
  listFolderUsers: connectorId => commands.listFolderUsers(connectorId),
  setCompanionPinned: pinned => commands.setCompanionPinned(pinned),
  sendCompanionMessage: (text, conversationId, scopeRequestId) =>
    commands.sendCompanionMessage(text, conversationId, scopeRequestId),
  setCompanionNotify: notify => commands.setCompanionNotify(notify),
  setCompanionNotifyResults: notify =>
    commands.setCompanionNotifyResults(notify),
  companionFocusRequest: requestId => commands.companionFocusRequest(requestId),

  getGatewayStatus: () => commands.getGatewayStatus(),
  makeGatewayReachable: () => commands.makeGatewayReachable(),
  listGatewayHolds: () => commands.listGatewayHolds(),
  decideGatewayHold: (taskId, approve, note) =>
    commands.decideGatewayHold(taskId, approve, note),
  listGatewayAssignments: () => commands.listGatewayAssignments(),
  listRegistryConnectors: refresh => commands.listRegistryConnectors(refresh),
  getConnectorConsent: (registryUrl, connectorId) =>
    commands.getConnectorConsent(registryUrl, connectorId),
  installRegistryConnector: (registryUrl, connectorId, apiKey) =>
    commands.installRegistryConnector(registryUrl, connectorId, apiKey),
  removeConnector: id => commands.removeConnector(id),
  scaffoldConnector: input => commands.scaffoldConnector(input),
  publishConnector: input => commands.publishConnector(input),
  closeGatewayAssignment: (taskId, done, note) =>
    commands.closeGatewayAssignment(taskId, done, note),
  getGatewayConfig: () => commands.getGatewayConfig(),
  setGatewayConfig: config => commands.setGatewayConfig(config),
  listGatewayCallers: () => commands.listGatewayCallers(),
  setCallerPolicy: policy => commands.setCallerPolicy(policy),
  removeCallerPolicy: callerInstance =>
    commands.removeCallerPolicy(callerInstance),
  getGatewayUsage: callerInstance => commands.getGatewayUsage(callerInstance),
  getGatewayManifest: () => commands.getGatewayManifest(),
  setGatewayManifest: manifest => commands.setGatewayManifest(manifest),
  gatewayChat: message => commands.gatewayChat(message),

  openExternal: url => openUrl(url),

  subscribe: handler => coreEvents.subscribe(handler),
})
