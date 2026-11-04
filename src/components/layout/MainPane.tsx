import { AgentView } from '@/components/views/AgentView'
import { CapabilityDetailView } from '@/components/views/CapabilityDetailView'
import { DelegationThreadView } from '@/components/views/DelegationThreadView'
import { RequestDetailView } from '@/components/views/RequestDetailView'
import { MemberProfileView } from '@/components/views/MemberProfileView'
import { OnboardingView } from '@/components/views/OnboardingView'
import { RunDetailView } from '@/components/views/RunDetailView'
import { RunHistoryView } from '@/components/views/RunHistoryView'
import { StarterPacksView } from '@/components/views/StarterPacksView'
import { StudioView } from '@/components/views/StudioView'
import { TriggerDetailView } from '@/components/views/TriggerDetailView'
import { WorkflowCanvas } from '@/components/workflow-builder'
import { WorkflowDetailView } from '@/components/views/WorkflowDetailView'
import { WorkflowProjectView } from '@/components/work/WorkflowProjectView'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { AgentsListView } from '@/components/views/AgentsListView'
import { AllActivityView } from '@/components/home/AllActivityView'
import { HomeView } from '@/components/home/HomeView'
import {
  AutomationsPage,
  LibraryPage,
  NetworkPage,
} from '@/components/views/NavPages'
import { SettingsView } from '@/components/settings/SettingsView'
import { MyNotesView } from '@/components/notes/MyNotesView'
import { LeaveGuardDialog } from './LeaveGuardDialog'
import { LlmKeyNotice } from './LlmKeyNotice'

/**
 * The current view. There's no docked message box: agents have their own
 * (on the agent page), Studio has its own, and Ctrl+K's "Ask Nandi" row or
 * Home's ask entry open Studio.
 */
export function MainPane() {
  const view = useNavigationStore(state => state.current)

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      {view.kind === 'studio' && <LlmKeyNotice />}
      <main className="min-h-0 flex-1">
        <CurrentView view={view} />
      </main>
      <LeaveGuardDialog />
    </div>
  )
}

function CurrentView({ view }: { view: View }) {
  switch (view.kind) {
    case 'home':
      return <HomeView welcome={view.welcome} />
    case 'home-activity':
      return <AllActivityView filters={view.filters} />
    case 'agents':
      return <AgentsListView />
    case 'automations':
      return <AutomationsPage view={view} />
    case 'library':
      return <LibraryPage view={view} />
    case 'network':
      return <NetworkPage view={view} />
    case 'settings':
      return <SettingsView view={view} />
    case 'notes':
      return <MyNotesView note={view.note} />
    case 'agent':
      return (
        <AgentView
          key={view.agentId}
          agentId={view.agentId}
          focusMessageId={view.focusMessageId}
          tab={view.tab}
        />
      )
    case 'workflow':
      return (
        <WorkflowDetailView
          key={view.workflowId}
          workflowId={view.workflowId}
          initialTab={view.tab}
        />
      )
    case 'trigger':
      return (
        <TriggerDetailView key={view.triggerId} triggerId={view.triggerId} />
      )
    case 'studio':
      return (
        <StudioView
          conversationId={view.conversationId}
          draft={view.draft === true}
        />
      )
    case 'run':
      return <RunHistoryView key={view.runId} runId={view.runId} />
    case 'onboarding':
      return <OnboardingView />
    case 'starter-packs':
      return <StarterPacksView />
    case 'run-detail':
      return (
        <RunDetailView key={view.runId} runId={view.runId} tab={view.tab} />
      )
    case 'workflow-builder':
      return (
        <WorkflowCanvas key={view.workflowId} workflowId={view.workflowId} />
      )
    case 'member-profile':
      return (
        <MemberProfileView key={view.instanceId} instanceId={view.instanceId} />
      )
    case 'delegation-thread':
      return <DelegationThreadView key={view.taskId} taskId={view.taskId} />
    case 'capability-detail':
      return <CapabilityDetailView key={view.id} capabilityId={view.id} />
    case 'work-item':
      return <WorkflowProjectView key={view.id} workId={view.id} />
    case 'request':
      return (
        <RequestDetailView
          key={view.requestId}
          requestId={view.requestId}
          stepId={view.stepId}
        />
      )
  }
}
