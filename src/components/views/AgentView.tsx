import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  ArrowLeft,
  Bot,
  Copy,
  MessageSquare,
  MoreHorizontal,
  PencilRuler,
  Play,
  RotateCcw,
  Square,
  Trash2,
  Upload,
  Users,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { AgentStatusPill } from '@/components/agent-console/AgentStatusIndicator'
import { ApprovalRequestCard } from '@/components/agent-console/ApprovalRequestCard'
import { AgentWarningBanner } from '@/components/agent-console/ContainerControls'
import { useContainerControl } from '@/components/agent-console/use-container-control'
import { effectiveStatus } from '@/components/agent-console/agent-status'
import { AgentComposer } from '@/components/agent-console/AgentComposer'
import { MessageList } from '@/components/agent-console/MessageList'
import { isWorking } from '@/components/agent-console/working'
import { TurnQueueStrip } from '@/components/agent-console/TurnQueue'
import { AgentOverviewTab } from '@/components/agents/AgentOverviewTab'
import { AgentRunsTab } from '@/components/agents/AgentRunsTab'
import { AgentSettingsTab } from '@/components/agents/AgentSettingsTab'
import { AgentPlaybooksTab } from '@/components/agents/AgentPlaybooksTab'
import { AgentTaskStrip } from '@/components/agents/AgentTaskStrip'
import { DeleteAgentDialog } from '@/components/agents/DeleteAgentDialog'
import {
  AGENT_TABS,
  runtimeDetail,
  runtimeName,
  type AgentTab,
} from '@/components/agents/agent-page-model'
import { PublishDialog } from '@/components/marketplace/PublishDialog'
import { SharedAgentBanner } from '@/components/marketplace/SharedAgentBanner'
import { AgentSharingDialog } from '@/components/sharing/AgentSharingDialog'
import { useAgentOverviews } from '@/hooks/useAgents'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { decideApproval } from '@/lib/agent-actions'
import { loadAgent } from '@/lib/agent-sync'
import { openAgentInStudio } from '@/lib/ask'
import type { ContainerState } from '@/lib/tauri-bindings'
import {
  useAgent,
  useCurrentRun,
  useMessages,
  usePendingApprovals,
} from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import type { AgentInfo } from '@/types/agent'
import { RefreshButton } from '@/components/live/RefreshButton'
import { UpdatedBy } from '@/components/live/UpdatedBy'
import { ViewHeader } from './ViewHeader'

/** "Updated just now by …": its definition, or a file in its work folder */
const AGENT_CHANGES = ['agent', 'work'] as const

interface AgentViewProps {
  agentId: string
  focusMessageId?: string
  tab?: AgentTab
}

/**
 * One agent's page: Overview (health, what's waiting, recent work), Chat,
 * Runs and Settings. Using it is here; changing its instructions, tools
 * and abilities is Studio's (Edit in Studio). The agent keeps running in
 * Rust whether or not this view is mounted.
 */
export function AgentView({ agentId, focusMessageId, tab }: AgentViewProps) {
  const { t } = useTranslation()
  const agentState = useAgent(agentId)

  // Agents created after boot have no snapshot yet
  useEffect(() => {
    if (!agentState) void loadAgent(agentId)
  }, [agentId, agentState])

  if (!agentState) {
    return (
      <div className="flex h-full flex-col">
        <div className="flex items-center gap-3 border-b px-4 py-3">
          <Skeleton className="size-9 rounded-full" />
          <div className="space-y-1.5">
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-3 w-64" />
          </div>
        </div>
        <div className="space-y-2 p-6">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      </div>
    )
  }

  if (!agentState.agent) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Bot />
          </EmptyMedia>
          <EmptyTitle>{t('views.agentNotFound')}</EmptyTitle>
          <EmptyDescription>
            {t('views.agentNotFoundDescription')}
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              useNavigationStore.getState().navigate({ kind: 'agents' })
            }
          >
            {t('views.backToAgents')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }

  return (
    <AgentPage
      agent={agentState.agent}
      container={agentState.container}
      warning={agentState.warning}
      focusMessageId={focusMessageId}
      // A deep link to a message opens the chat
      initialTab={tab ?? (focusMessageId ? 'chat' : 'overview')}
    />
  )
}

function AgentPage({
  agent,
  container,
  warning,
  focusMessageId,
  initialTab,
}: {
  agent: AgentInfo
  container: ContainerState
  warning: string | null
  focusMessageId?: string
  initialTab: AgentTab
}) {
  const { t } = useTranslation()
  const run = useCurrentRun(agent.id)
  const pending = usePendingApprovals(agent.id)
  const overviews = useAgentOverviews()
  const { data: defaultModel } = useDefaultModel()
  const [tab, setTab] = useState<AgentTab>(initialTab)
  // The Settings and Playbooks tabs report unsaved changes; leaving asks first
  const [editsDirty, setEditsDirty] = useState(false)
  const [leaveTo, setLeaveTo] = useState<AgentTab | null>(null)
  // A new deep link (e.g. "Chat" from the menu) switches the tab
  const [lastInitial, setLastInitial] = useState(initialTab)
  if (initialTab !== lastInitial) {
    setLastInitial(initialTab)
    setTab(initialTab)
  }

  const overview = overviews.data?.find(o => o.agentId === agent.id)
  const detail = overview
    ? runtimeDetail(
        overview,
        defaultModel?.effectiveModel ?? defaultModel?.defaultModel ?? null
      )
    : null
  const subtitle = [
    agent.description,
    overview && runtimeName(overview.runtime),
    detail?.kind === 'model'
      ? detail.label
      : detail && t('agentPage.where.container'),
  ]
    .filter(Boolean)
    .join(' · ')

  const changeTab = (next: AgentTab) => {
    if (next !== tab && editsDirty) {
      setLeaveTo(next)
      return
    }
    setTab(next)
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label={t('agentPage.back')}
              onClick={() =>
                useNavigationStore.getState().navigate({ kind: 'agents' })
              }
            >
              <ArrowLeft className="rtl:rotate-180" />
            </Button>
            <AgentAvatar agent={agent} className="size-8" />
          </div>
        }
        title={agent.name}
        titleAdornment={
          <AgentStatusPill status={effectiveStatus(agent, run)} />
        }
        subtitle={subtitle || t('agentPage.list.noDescription')}
        actions={
          <>
            <UpdatedBy
              areas={AGENT_CHANGES}
              targetId={agent.id}
              className="hidden md:block"
            />
            <RefreshButton />
            {agent.origin.kind === 'local' && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => openAgentInStudio(agent.id)}
              >
                <PencilRuler />
                {t('agentPage.editInStudio')}
              </Button>
            )}
            <AgentPageMenu
              agent={agent}
              container={container}
              onChat={() => changeTab('chat')}
            />
          </>
        }
      />

      <SharedAgentBanner
        agent={agent}
        onOpenRules={() => changeTab('settings')}
      />
      {warning && <AgentWarningBanner agentId={agent.id} warning={warning} />}
      {pending[0] && (
        <div className="border-b bg-warning/5 px-4 py-2">
          <ApprovalRequestCard
            message={{
              id: pending[0].id,
              tool: pending[0].tool,
              args: pending[0].args,
              status: 'pending',
              createdAt: pending[0].createdAt,
              expiresAt: pending[0].expiresAt,
            }}
            onDecide={(approvalId, decision) =>
              decideApproval(agent.id, approvalId, decision)
            }
          />
          {pending.length > 1 && (
            <Button
              variant="link"
              size="sm"
              className="h-auto px-0"
              onClick={() => changeTab('overview')}
            >
              {t('agentPage.banner.more', { count: pending.length - 1 })}
            </Button>
          )}
        </div>
      )}

      <Tabs
        value={tab}
        onValueChange={value => changeTab(value as AgentTab)}
        className="flex min-h-0 flex-1 flex-col gap-0"
      >
        <TabsList className="mx-4 mt-2 self-start">
          {AGENT_TABS.map(id => (
            <TabsTrigger key={id} value={id}>
              {t(`agentPage.tabs.${id}`)}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent
          value="overview"
          className="min-h-0 flex-1 overflow-y-auto"
        >
          <AgentOverviewTab
            agent={agent}
            overview={overview}
            container={container}
            onOpenTab={changeTab}
          />
        </TabsContent>
        <TabsContent value="chat" className="flex min-h-0 flex-1 flex-col">
          <AgentChat agent={agent} focusMessageId={focusMessageId} />
        </TabsContent>
        <TabsContent value="runs" className="min-h-0 flex-1 overflow-y-auto">
          <AgentRunsTab agent={agent} onChat={() => changeTab('chat')} />
        </TabsContent>
        <TabsContent
          value="settings"
          className="min-h-0 flex-1 overflow-y-auto"
        >
          <AgentSettingsTab agent={agent} onDirtyChange={setEditsDirty} />
        </TabsContent>
        <TabsContent
          value="playbooks"
          className="min-h-0 flex-1 overflow-y-auto"
        >
          <AgentPlaybooksTab agent={agent} onDirtyChange={setEditsDirty} />
        </TabsContent>
      </Tabs>

      <AlertDialog
        open={leaveTo !== null}
        onOpenChange={open => !open && setLeaveTo(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('agentPage.settings.discardTitle')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('agentPage.settings.discardDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('agentPage.settings.keepEditing')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setEditsDirty(false)
                if (leaveTo) setTab(leaveTo)
                setLeaveTo(null)
              }}
            >
              {t('agentPage.settings.discard')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** The Chat tab: today's agent console, with its own composer */
function AgentChat({
  agent,
  focusMessageId,
}: {
  agent: AgentInfo
  focusMessageId?: string
}) {
  const { t } = useTranslation()
  const messages = useMessages(agent.id)
  const run = useCurrentRun(agent.id)

  return (
    <>
      <TurnQueueStrip agentId={agent.id} agentName={agent.name} />
      <AgentTaskStrip agentId={agent.id} />
      <MessageList
        messages={messages}
        assistant={agent}
        working={isWorking(run, messages)}
        focusMessageId={focusMessageId}
        onDecide={(approvalId, decision) =>
          decideApproval(agent.id, approvalId, decision)
        }
        empty={
          <Empty className="h-full">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <MessageSquare />
              </EmptyMedia>
              <EmptyTitle>
                {t('agentPage.chat.emptyTitle', { name: agent.name })}
              </EmptyTitle>
              <EmptyDescription>
                {t('agentConsole.chat.emptyDescription', {
                  agentName: agent.name,
                })}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        }
      />
      <AgentComposer key={agent.id} agentId={agent.id} />
    </>
  )
}

/**
 * The page's "⋯": chat, publish, sharing, copy the ID, the container, and
 * delete. Always visible (never right-click only).
 */
function AgentPageMenu({
  agent,
  container,
  onChat,
}: {
  agent: AgentInfo
  container: ContainerState
  onChat: () => void
}) {
  const { t } = useTranslation()
  const control = useContainerControl(agent.id, container)
  const [publishing, setPublishing] = useState(false)
  const [sharing, setSharing] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [restarting, setRestarting] = useState(false)
  const mine = agent.origin.kind === 'local'
  const running = control.status === 'running'

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            size="icon"
            variant="outline"
            className="size-8"
            aria-label={t('agentMenu.label', { name: agent.name })}
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-60">
          <DropdownMenuItem onSelect={onChat}>
            <MessageSquare />
            {t('agentMenu.chat')}
          </DropdownMenuItem>
          {mine && (
            <DropdownMenuItem onSelect={() => setPublishing(true)}>
              <Upload />
              {t('marketplace.publish.button')}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => setSharing(true)}>
            <Users />
            {t('agentMenu.sharingSettings')}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              void navigator.clipboard
                .writeText(agent.id)
                .then(() => toast.success(t('agentPage.menu.copied')))
            }}
          >
            <Copy />
            {t('agentPage.menu.copyId')}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
            {t(`agentPage.container.${control.status}`)}
          </DropdownMenuLabel>
          {running ? (
            <>
              <DropdownMenuItem
                disabled={control.busy}
                onSelect={() => setRestarting(true)}
              >
                <RotateCcw />
                {t('agentPage.container.restart')}
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={control.busy}
                onSelect={() => control.run('stop')}
              >
                <Square />
                {t('agentPage.container.stop')}
              </DropdownMenuItem>
            </>
          ) : (
            <DropdownMenuItem
              disabled={control.busy || control.status === 'starting'}
              onSelect={() => control.run('start')}
            >
              <Play />
              {t('agentPage.container.start')}
            </DropdownMenuItem>
          )}
          {mine && !agent.builtin && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                variant="destructive"
                onSelect={() => setDeleting(true)}
              >
                <Trash2 />
                {t('agentPage.delete.action')}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {mine && (
        <PublishDialog
          agent={agent}
          open={publishing}
          onOpenChange={setPublishing}
        />
      )}
      <AgentSharingDialog
        agent={agent}
        open={sharing}
        onOpenChange={setSharing}
      />
      <DeleteAgentDialog
        agent={agent}
        open={deleting}
        onOpenChange={setDeleting}
      />
      <AlertDialog open={restarting} onOpenChange={setRestarting}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('agentPage.container.restartTitle')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('agentPage.container.restartDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('agentPage.container.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => control.run('restart')}>
              {t('agentPage.container.restart')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
