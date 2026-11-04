/**
 * Ask box helpers shared by its components and views.
 */
import type {
  ActionPlan,
  ArtifactRef,
  PlanStep,
  AskContext,
  AskSourceRef,
  EditPreview,
} from '@/lib/bindings'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { useStudioStore } from '@/store/studio-store'

/** Starts the last line of an answer, which lists follow-up questions
 * (`FOLLOWUPS_MARKER` in `src-tauri/src/services/ask/mod.rs`) */
export const FOLLOWUPS_MARKER = 'FOLLOWUPS:'

/** Where a source opens in the main pane, if anywhere */
export function sourceView(source: AskSourceRef): View | null {
  switch (source.kind) {
    case 'agent':
      return { kind: 'agent', agentId: source.id }
    case 'capability':
      return { kind: 'capability-detail', id: source.id }
    case 'workflow':
      return { kind: 'workflow', workflowId: source.id }
    case 'run':
      return { kind: 'run-detail', runId: source.id }
    case 'member':
      return { kind: 'member-profile', instanceId: source.id }
    case 'gateway':
      return { kind: 'network', tab: 'sharing', sharing: 'offer' }
    default:
      return null
  }
}

/** Plan steps that write, send or delegate (an agent turn, a workflow, a
 * delegation): a plan with any of them is shown before it runs */
const ACTING_STEPS = new Set<PlanStep['kind']>([
  'localAgent',
  'workflow',
  'capability',
])

/**
 * Whether the user sees the plan before it runs (work-model §5). Plans
 * that only point at people or manual steps start straight away.
 */
export function planNeedsApproval(plan: ActionPlan): boolean {
  return plan.steps.some(step => ACTING_STEPS.has(step.kind))
}

/** What the ask box knows about where it is */
export function askContextForView(view: View): AskContext {
  const context = (
    currentView: string,
    focusedArtifact: ArtifactRef | null = null
  ): AskContext => ({ currentView, focusedArtifact, recentRuns: [] })
  switch (view.kind) {
    case 'home':
      return context('home')
    case 'network':
      return view.tab === 'sharing' && (view.sharing ?? 'offer') === 'offer'
        ? context('gateway', { kind: 'gateway' })
        : context('network')
    case 'agent':
      return context(`agent:${view.agentId}`, {
        kind: 'agent',
        id: view.agentId,
      })
    case 'capability-detail':
      return context(`capability:${view.id}`, {
        kind: 'capability',
        id: view.id,
      })
    case 'workflow':
    case 'workflow-builder':
      return context(`workflow:${view.workflowId}`, {
        kind: 'workflow',
        id: view.workflowId,
      })
    case 'member-profile':
      return context(`member:${view.instanceId}`)
    case 'run':
    case 'run-detail':
      return { ...context(view.kind), recentRuns: [view.runId] }
    default:
      return context(view.kind)
  }
}

/** "Edit in Studio": Studio with the agent's file open beside the chat */
export function openAgentInStudio(agentId: string): void {
  const studio = useStudioStore.getState()
  studio.setTab('agent')
  studio.select('agent', agentId)
  useNavigationStore
    .getState()
    .navigate({ kind: 'studio', conversationId: null })
}

/** Edit in Studio for a workflow: its file open in Studio's editor */
export function openWorkflowInStudio(workflowId: string): void {
  const studio = useStudioStore.getState()
  studio.setTab('workflow')
  studio.select('workflow', workflowId)
  useNavigationStore
    .getState()
    .navigate({ kind: 'studio', conversationId: null })
}

/** Opens an approved edit in Studio's editor, ready to save */
export function openEditInStudio(preview: EditPreview): void {
  useStudioStore.getState().applyEdit(preview)
  useNavigationStore
    .getState()
    .navigate({ kind: 'studio', conversationId: null })
}
