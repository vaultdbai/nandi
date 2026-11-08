/** `NandiClient` over Tauri IPC: the generated tauri-specta bindings. */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { commands, type JsonValue } from '@/lib/tauri-bindings'
import { CORE_EVENT_NAMES, type NandiEvent } from './events'
import { marketplaceFilter } from './marketplace-filter'
import type { NandiClient } from './types'

export const tauriClient: NandiClient = {
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

  getAgentQueue: agentId => commands.getAgentQueue(agentId),
  getCurrentTurn: agentId => commands.getCurrentTurn(agentId),
  cancelQueuedRun: runId => commands.cancelQueuedRun(runId),
  reprioritizeQueuedRun: (runId, newPriority) =>
    commands.reprioritizeQueuedRun(runId, newPriority),

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

  loadPreferences: () => commands.loadPreferences(),
  savePreferences: preferences => commands.savePreferences(preferences),
  getLlmKeyStatus: () => commands.getLlmKeyStatus(),
  getDefaultModel: () => commands.getDefaultModel(),
  setDefaultModel: model => commands.setDefaultModel(model),
  setFreeTrial: enabled => commands.setFreeTrial(enabled),
  testLlmKey: (provider, apiKey) => commands.testLlmKey(provider, apiKey),
  listProviderModels: provider => commands.listProviderModels(provider),
  detectOllama: () => commands.detectOllama(),
  setLlmApiKey: (provider, apiKey) => commands.setLlmApiKey(provider, apiKey),
  saveEmergencyData: (filename, data) =>
    commands.saveEmergencyData(filename, data),
  loadEmergencyData: filename => commands.loadEmergencyData(filename),
  cleanupOldRecoveryFiles: () => commands.cleanupOldRecoveryFiles(),

  getEgressStatus: () => commands.getEgressStatus(),
  setAwsCredentials: (accessKey, secret) =>
    commands.setAwsCredentials(accessKey, secret),
  setAwsRegion: region => commands.setAwsRegion(region),
  getWorkflow: workflowId => commands.getWorkflow(workflowId),
  createWorkflow: input => commands.createWorkflow(input),
  runWorkflow: workflowId => commands.runWorkflow(workflowId),
  listWorkflowTemplates: () => commands.listWorkflowTemplates(),
  createWorkflowFromTemplate: input =>
    commands.createWorkflowFromTemplate(input),
  approveWorkflowStep: (runId, stepId, decision) =>
    commands.approveWorkflowStep(runId, stepId, decision),

  listTriggers: () => commands.listTriggers(),
  getTrigger: id => commands.getTrigger(id),
  createTrigger: input => commands.createTrigger(input),
  updateTrigger: (id, input) => commands.updateTrigger(id, input),
  deleteTrigger: id => commands.deleteTrigger(id),
  setTriggerEnabled: (id, enabled) => commands.setTriggerEnabled(id, enabled),
  rotateTriggerSecret: id => commands.rotateTriggerSecret(id),
  testFireTrigger: (id, payload) =>
    commands.testFireTrigger(id, (payload ?? null) as JsonValue | null),
  listTriggerFires: (triggerId, limit) =>
    commands.listTriggerFires(triggerId, limit),
  getWebOrigin: async () => {
    const result = await commands.getWebOrigin()
    return result.status === 'ok' ? result.data : null
  },
  previewCron: schedule => commands.previewCron(schedule),
  previewCalendar: config =>
    commands.previewCalendar(
      config as Parameters<typeof commands.previewCalendar>[0]
    ),

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

  listPilots: () => commands.listPilots(),
  installPilot: id => commands.installPilot(id),
  startPilot: id => commands.startPilot(id),
  stopPilot: id => commands.stopPilot(id),
  pilotStatus: id => commands.pilotStatus(id),
  grantPilotCapability: (agentId, pilotId, capability) =>
    commands.grantPilotCapability(agentId, pilotId, capability),
  revokePilotCapability: (agentId, pilotId, capability) =>
    commands.revokePilotCapability(agentId, pilotId, capability),
  agentPilotPermissions: agentId => commands.agentPilotPermissions(agentId),
  setPilotSecret: (agentId, pilotId, name, value) =>
    commands.setPilotSecret(agentId, pilotId, name, value),
  addPilotSecret: (agentId, pilotId, name, value) =>
    commands.addPilotSecret(agentId, pilotId, name, value),
  clearBrowserProfile: (agentId, pilotId) =>
    commands.clearBrowserProfile(agentId, pilotId),
  agentPolicies: agentId => commands.agentPolicies(agentId),
  listPolicies: () => commands.listPolicies(),

  listAllRuns: (filters, limit, offset) =>
    commands.listAllRuns(filters, limit, offset),
  getRunStats: range => commands.getRunStats(range),
  listAuditLog: (kind, limit, offset) =>
    commands.listAuditLog(kind, limit, offset),

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

  findAgentsByCapability: capability =>
    commands.findAgentsByCapability(capability),
  listDeclaredCapabilities: () => commands.listDeclaredCapabilities(),
  getAgentCard: agentId => commands.getAgentCard(agentId),

  getRelayStatus: () => commands.getRelayStatus(),
  setRelayUrl: url => commands.setRelayUrl(url),
  joinRelay: (url, joinCode) => commands.joinRelay(url, joinCode),
  getRelayInvite: () => commands.getRelayInvite(),
  listOnlineAgents: capability => commands.listOnlineAgents(capability),
  testRelayConnection: url => commands.testRelayConnection(url),
  getRelayInstances: () => commands.getRelayInstances(),

  listAvailableProviders: capability =>
    commands.listAvailableProviders(capability),
  delegateTask: (capability, input, target) =>
    commands.delegateTask(capability, input, target),
  listDelegationHistory: limit => commands.listDelegationHistory(limit),
  getAgentDelegation: agentId => commands.getAgentDelegation(agentId),
  setAgentDelegation: (agentId, settings) =>
    commands.setAgentDelegation(agentId, settings),

  listTeams: () => commands.listTeams(),
  getTeam: teamId => commands.getTeam(teamId),
  createTeam: registryUrl => commands.createTeam(registryUrl),
  joinTeam: teamId => commands.joinTeam(teamId),
  leaveTeam: teamId => commands.leaveTeam(teamId),
  syncTeamRegistry: teamId => commands.syncTeamRegistry(teamId),
  shareAgent: (teamId, agentId) => commands.shareAgent(teamId, agentId),
  unshareAgent: (teamId, agentId) => commands.unshareAgent(teamId, agentId),

  listAgentsSharedWithMe: () => commands.listAgentsSharedWithMe(),
  listAgentsSharedByMe: () => commands.listAgentsSharedByMe(),
  listCapabilityProviders: capability =>
    commands.listCapabilityProviders(capability),
  setProviderPin: (capability, cardId) =>
    commands.setProviderPin(capability, cardId),
  listTrustPolicies: () => commands.listTrustPolicies(),
  setTrustPolicy: (requester, providerAgent, policy) =>
    commands.setTrustPolicy(requester, providerAgent, policy),

  analyzeRun: (runId, question) => commands.analyzeRun(runId, question),
  summarizeRun: runId => commands.summarizeRun(runId),

  getWorkflowGraph: workflowId => commands.getWorkflowGraph(workflowId),
  saveWorkflowYaml: (workflowId, yaml) =>
    commands.saveWorkflowYaml(workflowId, yaml),

  searchMarketplace: query => commands.searchMarketplace(query),

  getCompanionSnapshot: dayStart => commands.getCompanionSnapshot(dayStart),
  toggleCompanionWindow: () => commands.toggleCompanionWindow(),
  quitNandi: () => commands.quitNandi(),
  getImageStatus: image => commands.getImageStatus(image),
  buildImage: image => commands.buildImage(image),
  pullImage: image => commands.pullImage(image),
  getToolsStatus: bundle => commands.getToolsStatus(bundle),
  installTools: bundle => commands.installTools(bundle),
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

  searchPalette: query => commands.searchPalette(query),
  executePaletteAction: (actionId, args) =>
    commands.executePaletteAction(actionId, args),

  loadAgentMemory: agentId => commands.loadAgentMemory(agentId),
  updateAgentMemory: (agentId, index, content) =>
    commands.updateAgentMemory(agentId, index, content),
  deleteAgentMemory: (agentId, index) =>
    commands.deleteAgentMemory(agentId, index),
  refreshAgentMemory: agentId => commands.refreshAgentMemory(agentId),
  getAgentMemoryEnabled: agentId => commands.getAgentMemoryEnabled(agentId),
  setAgentMemoryEnabled: (agentId, enabled) =>
    commands.setAgentMemoryEnabled(agentId, enabled),

  getIdentityStatus: () => commands.getIdentityStatus(),
  setDisplayName: name => commands.setDisplayName(name),
  setUserSlug: slug => commands.setUserSlug(slug),
  setOnboardingStep: step => commands.setOnboardingStep(step),

  getGithubSignIn: () => commands.getGithubSignIn(),
  startGithubSignIn: () => commands.startGithubSignIn(),
  pollGithubSignIn: () => commands.pollGithubSignIn(),
  signOutGithub: () => commands.signOutGithub(),

  getRemoteStatus: () => commands.getRemoteStatus(),
  configureRemote: (url, credential, prefix, slug) =>
    commands.configureRemote(url, credential, prefix, slug),
  clearRemote: () => commands.clearRemote(),
  testRemoteConnection: () => commands.testRemoteConnection(),
  pushToRemote: overwrite => commands.pushToRemote(overwrite),
  pullFromRemote: () => commands.pullFromRemote(),
  listRemoteBranches: () => commands.listRemoteBranches(),

  getTask: taskId => commands.getTask(taskId),
  listTasks: filter => commands.listTasks(filter),
  cancelTask: (taskId, reason) => commands.cancelTask(taskId, reason),
  queryTaskStatus: taskId => commands.queryTaskStatus(taskId),
  getTaskEvents: taskId => commands.getTaskEvents(taskId),
  getTaskMessages: taskId => commands.getTaskMessages(taskId),
  getEtaEstimate: (capability, providerAgent) =>
    commands.getEtaEstimate(capability, providerAgent),
  getRequest: id => commands.getRequest(id),
  requestForRun: runId => commands.requestForRun(runId),
  retryTask: id => commands.retryTask(id),
  startRequest: (plan, text) => commands.startRequest(plan, text),
  completeRequestStep: (id, position) =>
    commands.completeRequestStep(id, position),
  getChiefOfStaffDigest: () => commands.getChiefOfStaffDigest(),
  markChiefOfStaffDigestSeen: () => commands.markChiefOfStaffDigestSeen(),
  approveQueuedWork: queueId => commands.approveQueuedWork(queueId),
  dismissQueuedWork: queueId => commands.dismissQueuedWork(queueId),
  getAutoApproveRules: () => commands.getAutoApproveRules(),
  setAutoApproveRules: rules => commands.setAutoApproveRules(rules),
  getIntakeThread: conversationId => commands.getIntakeThread(conversationId),
  sendIntakeMessage: (conversationId, text, clientMessageId) =>
    commands.sendIntakeMessage(conversationId, text, clientMessageId),
  cancelRequest: id => commands.cancelRequest(id),
  rerunRequest: id => commands.rerunRequest(id),
  mascotGet: () => commands.mascotGet(),
  mascotSet: name => commands.mascotSet(name),
  mascotPhrase: input => commands.mascotPhrase(input),
  draftWorkflowFromRequest: id => commands.draftWorkflowFromRequest(id),

  respondToClarification: response => commands.respondToClarification(response),
  escalateClarification: (taskId, questionId) =>
    commands.escalateClarification(taskId, questionId),
  getPendingClarifications: () => commands.getPendingClarifications(),
  getClarificationPolicy: agentId => commands.getClarificationPolicy(agentId),
  setClarificationPolicy: (agentId, policy) =>
    commands.setClarificationPolicy(agentId, policy),

  getGatewayStatus: () => commands.getGatewayStatus(),
  makeGatewayReachable: () => commands.makeGatewayReachable(),
  listGatewayHolds: () => commands.listGatewayHolds(),
  decideGatewayHold: (taskId, approve) =>
    commands.decideGatewayHold(taskId, approve),
  listGatewayAssignments: () => commands.listGatewayAssignments(),
  getMyNotes: () => commands.getMyNotes(),
  readNote: name => commands.readNote(name),
  saveNote: (name, content) => commands.saveNote(name, content),
  deleteNote: name => commands.deleteNote(name),
  setNotesAgentsCanWrite: allow => commands.setNotesAgentsCanWrite(allow),
  listRecentChanges: () => commands.listRecentChanges(),
  refreshFromDisk: () => commands.refreshFromDisk(),
  getEmailStatus: () => commands.getEmailStatus(),
  startEmailSignIn: provider => commands.startEmailSignIn(provider),
  pollEmailSignIn: provider => commands.pollEmailSignIn(provider),
  checkEmailNow: () => commands.checkEmailNow(),
  setEmailPaused: (provider, paused) =>
    commands.setEmailPaused(provider, paused),
  disconnectEmail: provider => commands.disconnectEmail(provider),
  setEmailSettings: settings => commands.setEmailSettings(settings),
  setEmailBackfill: (provider, action) =>
    commands.setEmailBackfill(provider, action),
  estimateEmailBackfill: range => commands.estimateEmailBackfill(range),
  listConnectorGrants: () => commands.listConnectorGrants(),
  grantConnectorAccess: (agentId, connectorId, access, scope) =>
    commands.grantConnectorAccess(agentId, connectorId, access, scope),
  revokeConnectorAccess: grant => commands.revokeConnectorAccess(grant),
  answerAccessRequest: (approvalId, choice, subdomains = false) =>
    commands.answerAccessRequest(approvalId, choice, subdomains),
  listRegistryConnectors: refresh => commands.listRegistryConnectors(refresh),
  getConnectorConsent: (registryUrl, connectorId) =>
    commands.getConnectorConsent(registryUrl, connectorId),
  installRegistryConnector: (registryUrl, connectorId, apiKey) =>
    commands.installRegistryConnector(registryUrl, connectorId, apiKey),
  removeConnector: id => commands.removeConnector(id),
  scaffoldConnector: input => commands.scaffoldConnector(input),
  publishConnector: input => commands.publishConnector(input),
  getWorkspaceStatus: () => commands.getWorkspaceStatus(),
  startWorkspaceSignIn: kind => commands.startWorkspaceSignIn(kind),
  pollWorkspaceSignIn: kind => commands.pollWorkspaceSignIn(kind),
  disconnectWorkspace: kind => commands.disconnectWorkspace(kind),
  checkCalendarNow: () => commands.checkCalendarNow(),
  setCalendarInterval: minutes => commands.setCalendarInterval(minutes),
  listNoteSuggestions: () => commands.listNoteSuggestions(),
  answerNoteSuggestion: (id, accept) =>
    commands.answerNoteSuggestion(id, accept),
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

  getNetworkSnapshot: () => commands.getNetworkSnapshot(),
  refreshNetwork: () => commands.refreshNetwork(),

  getTeamRoster: teamId => commands.getTeamRoster(teamId),
  getMemberProfile: (teamId, handle) =>
    commands.getMemberProfile(teamId, handle),
  getMemberByInstance: instanceId => commands.getMemberByInstance(instanceId),
  acceptPeerKey: instanceId => commands.acceptPeerKey(instanceId),
  listGroups: () => commands.listGroups(),
  getGroup: groupId => commands.getGroup(groupId),
  createGroup: (name, description, policy) =>
    commands.createGroup(name, description, policy),
  updateGroup: (groupId, name, description) =>
    commands.updateGroup(groupId, name, description),
  setGroupPolicy: (groupId, policy) => commands.setGroupPolicy(groupId, policy),
  inviteToGroup: (groupId, invite) => commands.inviteToGroup(groupId, invite),
  revokeGroupInvite: code => commands.revokeGroupInvite(code),
  joinGroup: code => commands.joinGroup(code),
  leaveGroup: groupId => commands.leaveGroup(groupId),
  removeGroupMember: (groupId, instanceId) =>
    commands.removeGroupMember(groupId, instanceId),
  setGroupRole: (groupId, instanceId, role) =>
    commands.setGroupRole(groupId, instanceId, role),
  getGroupAccess: groupId => commands.getGroupAccess(groupId),
  setGroupAccess: (groupId, level) => commands.setGroupAccess(groupId, level),
  setTrustRelationship: (instanceId, update) =>
    commands.setTrustRelationship(instanceId, update),
  getRelayTrust: () => commands.getRelayTrust(),
  setRelayTrust: enabled => commands.setRelayTrust(enabled),
  blockAgentFromMember: (instanceId, agentId) =>
    commands.blockAgentFromMember(instanceId, agentId),
  unblockAgentFromMember: (instanceId, agentId) =>
    commands.unblockAgentFromMember(instanceId, agentId),
  pauseDelegationsFromMember: (instanceId, paused) =>
    commands.pauseDelegationsFromMember(instanceId, paused),
  listTrustRequests: () => commands.listTrustRequests(),
  respondTrustRequest: (instanceId, answer, block) =>
    commands.respondTrustRequest(instanceId, answer, block),

  listCapabilities: () => commands.listCapabilities(),
  getCapability: id => commands.getCapability(id),
  saveCapability: definition => commands.saveCapability(definition),
  deleteCapability: id => commands.deleteCapability(id),
  transferCapabilityOwnership: (id, newOwner, reason) =>
    commands.transferCapabilityOwnership(id, newOwner, reason),
  listProvidersForCapability: id => commands.listProvidersForCapability(id),
  getCapabilityEvidence: (id, windowDays) =>
    commands.getCapabilityEvidence(id, windowDays),
  getCapabilityProviderBreakdown: id =>
    commands.getCapabilityProviderBreakdown(id),
  listCapabilityAmbiguities: id => commands.listCapabilityAmbiguities(id),
  promoteAmbiguityToSchema: (id, clusterId) =>
    commands.promoteAmbiguityToSchema(id, clusterId),

  listTools: () => commands.listTools(),
  getTool: toolId => commands.getTool(toolId),
  setToolEnabled: (toolId, enabled) => commands.setToolEnabled(toolId, enabled),
  getToolUsageStats: () => commands.getToolUsageStats(),

  listCapabilityRegistries: () => commands.listCapabilityRegistries(),
  addCapabilityRegistry: (url, kind, branchPrefix, userSlug) =>
    commands.addCapabilityRegistry(url, kind, branchPrefix, userSlug),
  removeCapabilityRegistry: url => commands.removeCapabilityRegistry(url),
  syncCapabilityRegistry: url => commands.syncCapabilityRegistry(url),

  queryCapabilityPresence: capabilityId =>
    commands.queryCapabilityPresence(capabilityId),

  listHarnesses: () => commands.listHarnesses(),
  detectHarnesses: () => commands.detectHarnesses(),
  getHarnessHealth: harnessId => commands.getHarnessHealth(harnessId),
  testHarness: harnessId => commands.testHarness(harnessId),
  setHarnessDefault: (harnessId, model) =>
    commands.setHarnessDefault(harnessId, model),
  clearHarnessDefault: () => commands.clearHarnessDefault(),
  getHarnessDefault: () => commands.getHarnessDefault(),

  listHarnessSessions: (agentId, limit) =>
    commands.listHarnessSessions(agentId, limit),
  getHarnessSession: sessionId => commands.getHarnessSession(sessionId),
  getHarnessEvents: (sessionId, afterSeq) =>
    commands.getHarnessEvents(sessionId, afterSeq),
  killHarnessSession: sessionId => commands.killHarnessSession(sessionId),

  detectAwsCli: () => commands.detectAwsCli(),
  listAwsProfiles: () => commands.listAwsProfiles(),
  getAwsProfile: name => commands.getAwsProfile(name),
  startAwsSsoLogin: profile => commands.startAwsSsoLogin(profile),
  pollAwsSsoLogin: loginId => commands.pollAwsSsoLogin(loginId),
  saveAwsKeyProfile: input => commands.saveAwsKeyProfile(input),
  startAwsSsoSetup: input => commands.startAwsSsoSetup(input),
  pollAwsSsoSetup: setupId => commands.pollAwsSsoSetup(setupId),
  finishAwsSsoSetup: finish => commands.finishAwsSsoSetup(finish),
  cancelAwsSsoSetup: setupId => commands.cancelAwsSsoSetup(setupId),
  checkAwsCredentials: (profile, region) =>
    commands.checkAwsCredentials(profile, region),
  getAwsBedrockApiKeyStatus: () => commands.getAwsBedrockApiKeyStatus(),

  listBedrockModels: region => commands.listBedrockModels(region),
  refreshBedrockModels: region => commands.refreshBedrockModels(region),
  startLitellmGateway: model => commands.startLitellmGateway(model),
  stopLitellmGateway: () => commands.stopLitellmGateway(),
  getLitellmGatewayStatus: () => commands.getLitellmGatewayStatus(),

  ask: request => commands.ask(request),
  askStreaming: request => commands.askStreaming(request),
  listAskHistory: limit => commands.listAskHistory(limit),

  loadArtifact: artifact => commands.loadArtifact(artifact),
  setStudioOpenArtifact: artifact => commands.setStudioOpenArtifact(artifact),
  saveArtifact: save => commands.saveArtifact(save),
  previewEdit: (artifact, instruction) =>
    commands.previewEdit(artifact, instruction),
  listPushTargets: () => commands.listPushTargets(),
  validateArtifact: (artifact, content) =>
    commands.validateArtifact(artifact, content),

  listWork: stateFilter => commands.listWork(stateFilter),
  getWorkItem: workId => commands.getWorkItem(workId),
  pauseWork: workId => commands.pauseWork(workId),
  resumeWork: workId => commands.resumeWork(workId),
  cancelWork: workId => commands.cancelWork(workId),
  rateWork: (workId, rating) => commands.rateWork(workId, rating),
  dismissWork: workId => commands.dismissWork(workId),
  listRequestReturns: () => commands.listRequestReturns(),
  dismissRequestReturn: taskId => commands.dismissRequestReturn(taskId),
  listDismissedWork: () => commands.listDismissedWork(),

  reassignTask: request => commands.reassignTask(request),
  listReassignCandidates: taskId => commands.listReassignCandidates(taskId),
  completeManualStep: (runId, stepId, completion) =>
    commands.completeManualStep(runId, stepId, completion),
  listPendingManualSteps: () => commands.listPendingManualSteps(),
  getRepoLlmConfig: () => commands.getRepoLlmConfig(),
  reloadRepoLlmConfig: () => commands.reloadRepoLlmConfig(),

  openExternal: url => openUrl(url),

  subscribe(handler) {
    const subscriptions: Promise<UnlistenFn>[] = CORE_EVENT_NAMES.map(name =>
      listen(name, e =>
        handler({ event: name, payload: e.payload } as NandiEvent)
      )
    )
    return {
      ready: Promise.all(subscriptions).then(() => undefined),
      unsubscribe: () => {
        for (const subscription of subscriptions) {
          void subscription.then(unlisten => unlisten())
        }
      },
    }
  },
}
