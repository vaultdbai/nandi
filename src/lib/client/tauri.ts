/** `NandiClient` over Tauri IPC: the generated tauri-specta bindings. */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { commands } from '@/lib/tauri-bindings'
import { CORE_EVENT_NAMES, type NandiEvent } from './events'
import { createEventBus } from './event-bus'
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
  gitStatus: () => commands.repoGitStatus(),
  gitDiff: path => commands.repoGitDiff(path),
  gitCommit: message => commands.repoGitCommit(message),

  getCompanionSnapshot: dayStart => commands.getCompanionSnapshot(dayStart),
  toggleCompanionWindow: () => commands.toggleCompanionWindow(),
  quitNandi: () => commands.quitNandi(),
  placeCompanion: shape => commands.placeCompanion(shape),
  setCompanionBubble: bubble => commands.setCompanionBubble(bubble),
  setCompanionStartBisonOnly: on => commands.setCompanionStartBisonOnly(on),
  setCompanionPinned: pinned => commands.setCompanionPinned(pinned),
  sendCompanionMessage: (text, conversationId, scopeRequestId) =>
    commands.sendCompanionMessage(text, conversationId, scopeRequestId),
  setCompanionNotify: notify => commands.setCompanionNotify(notify),
  setCompanionNotifyResults: notify =>
    commands.setCompanionNotifyResults(notify),
  companionFocusRequest: requestId => commands.companionFocusRequest(requestId),

  openExternal: url => openUrl(url),

  subscribe: handler => coreEvents.subscribe(handler),
})
