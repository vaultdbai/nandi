import '@testing-library/jest-dom'
import { vi } from 'vitest'
import type * as TauriCore from '@tauri-apps/api/core'

// Mock matchMedia for tests
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation(query => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(), // deprecated
    removeListener: vi.fn(), // deprecated
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
})

// jsdom lacks ResizeObserver (used by Radix ScrollArea and friends)
globalThis.ResizeObserver ??= class {
  observe = vi.fn()
  unobserve = vi.fn()
  disconnect = vi.fn()
}

// Tests run as the desktop app: `client` is the Tauri client over the
// mocked bindings below
vi.mock('@tauri-apps/api/core', async importOriginal => ({
  ...(await importOriginal<typeof TauriCore>()),
  isTauri: () => true,
}))

// Tauri events: controllable via emitMockEvent() in ./mock-events
vi.mock('@tauri-apps/api/event', async () => {
  const { mockListen } = await import('./mock-events')
  return {
    listen: vi.fn(mockListen),
    emit: vi.fn().mockResolvedValue(undefined),
  }
})

vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('@tauri-apps/plugin-updater', () => ({
  check: vi.fn().mockResolvedValue(null),
}))

// Mock typed Tauri bindings (tauri-specta generated)
vi.mock('@/lib/tauri-bindings', () => {
  const ok = (data: unknown) =>
    vi.fn().mockResolvedValue({ status: 'ok', data })
  return {
    commands: {
      loadPreferences: ok({ theme: 'system' }),
      savePreferences: ok(null),
      sendNativeNotification: ok(null),
      saveEmergencyData: ok(null),
      loadEmergencyData: ok(null),
      cleanupOldRecoveryFiles: ok(0),
      listAgents: ok([]),
      listAgentOverviews: ok([]),
      setLlmApiKey: ok(null),
      getLlmKeyStatus: ok([]),
      testLlmKey: ok({ ok: true, latencyMs: 5, failure: null, detail: null }),
      detectOllama: ok({ running: false, models: [] }),
      // Streams the way Rust does: the reply, then ask://completed
      askStreaming: vi.fn(async (request: unknown) => {
        const { commands } = await import('@/lib/tauri-bindings')
        const { emitMockEvent } = await import('./mock-events')
        const result = (await commands.ask(request as never)) as {
          status: string
          data?: unknown
          error?: string
        }
        const requestId = crypto.randomUUID()
        setTimeout(() =>
          emitMockEvent('ask://completed', {
            requestId,
            response: result.status === 'ok' ? result.data : null,
            error: result.status === 'ok' ? null : result.error,
          })
        )
        return { status: 'ok', data: requestId }
      }),
      listAskHistory: ok([]),
      ask: ok({
        kind: 'question',
        data: { answer: 'All quiet.', sources: [], followups: [] },
      }),
      getDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: 'claude:claude-sonnet-5',
      }),
      setDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: 'claude:claude-sonnet-5',
      }),
      setFreeTrial: ok({
        defaultModel: 'deepseek:deepseek-flash',
        effectiveModel: 'deepseek:deepseek-flash',
      }),
      setStudioOpenArtifact: ok(null),
      sendMessage: ok({
        runId: 'run-1',
        messageId: 'msg-1',
        conversationTitle: null,
        queued: false,
      }),
      sendStudioMessage: ok({
        runId: 'run-1',
        messageId: 'msg-1',
        conversationTitle: null,
        queued: false,
      }),
      createStudioConversation: ok({
        id: 'studio-new',
        title: '',
        createdAt: 0,
        updatedAt: 0,
        lastMessage: null,
      }),
      createAgent: ok(null),
      startAgent: ok('container-1'),
      stopAgent: ok(null),
      restartAgent: ok('container-1'),
      deleteAgent: ok(null),
      approveAction: ok(null),
      answerHandOver: ok(null),
      listPendingApprovals: ok([]),
      listWork: ok([]),
      listPendingManualSteps: ok([]),
      getPendingClarifications: ok([]),
      listDismissedWork: ok([]),
      // Home reads runs, delegations and holds
      listAllRuns: ok({ rows: [], total: 0 }),
      listDelegationHistory: ok([]),
      listGatewayHolds: ok([]),
      listGatewayAssignments: ok([]),
      getMyNotes: ok({
        path: '',
        managed: true,
        agentsCanWrite: true,
        notes: [],
      }),
      dismissWork: ok(null),
      cancelTurn: ok(null),
      getAgentQueue: ok([]),
      getCurrentTurn: ok(null),
      cancelQueuedRun: ok(null),
      reprioritizeQueuedRun: ok(null),
      listTriggers: ok([]),
      getTrigger: ok(null),
      createTrigger: ok(null),
      updateTrigger: ok(null),
      deleteTrigger: ok(null),
      setTriggerEnabled: ok(null),
      rotateTriggerSecret: ok(null),
      testFireTrigger: ok({
        id: 'fire-1',
        triggerId: 'trigger-1',
        firedAt: 0,
        payload: { test: true },
        runId: 'run-1',
        status: 'queued',
        error: null,
      }),
      listTriggerFires: ok([]),
      previewCron: ok({ valid: true, error: null, next: [] }),
      previewCalendar: ok({ valid: true, error: null, next: [] }),
      listWorkflowTemplates: ok([]),
      createWorkflowFromTemplate: ok(null),
      getWebOrigin: ok(null),
      listAvailablePacks: ok([]),
      listMarketplacePacks: ok([]),
      listAvailableAgents: ok([]),
      getAgentDetail: ok(null),
      getPackDetail: ok(null),
      installPack: ok(null),
      installAgent: ok(null),
      listInstalled: ok([]),
      uninstallInstalled: ok(null),
      forkInstalledAgent: ok(null),
      checkForUpdates: ok([]),
      applyUpdate: ok(null),
      getAgentConsent: ok({
        agentId: 'a',
        agentName: 'A',
        required: false,
        satisfied: true,
        categories: [],
      }),
      grantAgentConsent: ok(null),
      listRegistries: ok([]),
      addRegistry: ok(null),
      removeRegistry: ok(null),
      createCommunityRegistry: ok(null),
      publishAgent: ok(null),
      pickAttachments: ok([]),
      discardAttachment: ok(null),
      getAgentSnapshot: vi.fn().mockResolvedValue({
        status: 'error',
        error: 'no snapshot in tests',
      }),
      getDashboardSnapshot: ok({
        stats: {
          needsAttention: 0,
          runningAgents: 0,
          runsToday: 0,
          failedLastDay: 0,
        },
        triggerStats: { enabled: 0, firedToday: 0 },
        recentTriggerFires: [],
        attentionItems: [],
        recentActivity: [],
        agentHealth: [],
        recentOutputs: [],
      }),
      getRunDetail: vi.fn().mockResolvedValue({
        status: 'error',
        error: 'Run not found',
      }),
      getWorkflows: ok([]),
      getWorkflow: vi.fn().mockResolvedValue({
        status: 'error',
        error: 'Workflow not found',
      }),
      createWorkflow: vi.fn().mockResolvedValue({
        status: 'error',
        error: 'Invalid workflow YAML',
      }),
      runWorkflow: ok('workflow-run-1'),
      approveWorkflowStep: ok(null),
      getStudioConversations: ok([]),
      getConversation: ok({
        conversationId: '',
        messages: [],
        currentRun: null,
      }),
      getGithubSignIn: ok({ signedIn: false, login: null }),
      // Onboarded by default, so App renders the workspace shell
      getIdentityStatus: ok({
        onboarded: true,
        displayName: 'Test User',
        userSlug: 'test-user',
        branch: null,
        onboardingStep: 'done',
      }),
      setOnboardingStep: ok(null),
      validateArtifact: ok({ ok: true, errors: [], warnings: [] }),
      requestForRun: ok(null),
      repoStatus: ok({
        root: '/home/test/.nandi/repo',
        agents: 0,
        workflows: 0,
        triggers: 0,
        problems: [],
        watching: true,
      }),
      repoGitStatus: ok({ branch: 'main', ahead: 0, behind: 0, files: [] }),
    },
    unwrapResult: vi.fn((result: { status: string; data?: unknown }) => {
      if (result.status === 'ok') return result.data
      throw result
    }),
  }
})
