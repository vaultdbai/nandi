import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ArrowLeft,
  ChevronRight,
  MessageSquare,
  Pin,
  PinOff,
  Settings2,
} from 'lucide-react'
import still from '@/assets/brand/mascot/companion-bison-still.webp'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Empty, EmptyHeader, EmptyTitle } from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Spinner } from '@/components/ui/spinner'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { MessageList } from '@/components/agent-console/MessageList'
import { isWorking } from '@/components/agent-console/working'
import { AskBox } from '@/components/ask/AskBox'
import { mentionTarget } from '@/components/ask/mention-model'
import { labelText } from '@/components/live-runs/live-run-model'
import { LiveRunsSection } from '@/components/live-runs/LiveRunsSection'
import { LiveBadge, LiveStop } from '@/components/live-runs/LiveRunView'
import { ViewHeader } from '@/components/views/ViewHeader'
import { useAgents } from '@/hooks/useAgents'
import { useChats } from '@/hooks/useChats'
import { decideApproval, sendChatMessage, startChat } from '@/lib/agent-actions'
import { defaultAgentName, displayAgent } from '@/lib/agent-identity'
import { loadConversation } from '@/lib/agent-sync'
import { askContextForView, openEditInStudio } from '@/lib/ask'
import {
  DEFAULT_AGENT,
  type AgentInfo,
  type AttachmentInfo,
  type ChatSummary,
} from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { useConversation } from '@/store/agent-store'
import { useChatPinsStore } from '@/store/chat-pins-store'
import { useNavigationStore } from '@/store/navigation-store'
import { chatAgentId, chatLabel, recentChats } from './nandi-home-model'

/** The sitting bison, Nandi's mark (decorative: the caller labels it) */
export function NandiMark({ className }: { className?: string }) {
  return (
    <img
      src={still}
      alt=""
      aria-hidden
      draggable={false}
      className={className ?? 'size-8 object-contain'}
    />
  )
}

/**
 * One place to talk. Without a chat: the home (a composer, Live, Recent).
 * With one: that chat, its own thread of runs with its agent.
 */
export function NandiView({ conversationId }: { conversationId?: string }) {
  return conversationId ? (
    <ChatView key={conversationId} conversationId={conversationId} />
  ) : (
    <NandiHome />
  )
}

/**
 * The composer each chat ends with, in chat mode. `@` lists the person's
 * agents: a message starting `@Researcher …` starts a chat with that
 * agent instead.
 */
function Composer({
  placeholder,
  agentId,
  onSend,
}: {
  placeholder: string
  /** The chat's own agent: mentioning it just continues this chat */
  agentId?: string
  onSend: (text: string, attachments: AttachmentInfo[]) => void
}) {
  const view = useNavigationStore(state => state.current)
  const agents = useAgents().data ?? []
  const mentions = agents.map(agent => ({ id: agent.id, name: agent.name }))
  return (
    <AskBox
      className="mx-auto w-full max-w-3xl"
      context={askContextForView(view)}
      placeholder={placeholder}
      onApplyEdit={openEditInStudio}
      fixedMode="chat"
      chat={{
        placeholder,
        mentions,
        onSend: (text, attachments) => {
          const target = mentionTarget(text, mentions)
          if (target && target.message && target.agentId === agentId) {
            onSend(target.message, attachments)
          } else if (target && target.message) {
            void startChat(target.agentId, target.message, attachments)
          } else {
            onSend(text, attachments)
          }
        },
      }}
    />
  )
}

function NandiHome() {
  const { t } = useTranslation()
  const agents = useAgents().data ?? []
  const name = defaultAgentName(agents)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={<NandiMark />}
        title={name}
        subtitle={t('nandiHome.subtitle')}
        actions={
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              useNavigationStore
                .getState()
                .navigate({ kind: 'agent', agentId: DEFAULT_AGENT })
            }
          >
            <Settings2 />
            {t('nandiHome.settings', { name })}
          </Button>
        }
      />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 p-4">
          <Composer
            placeholder={t('nandiHome.composer')}
            onSend={(text, attachments) =>
              void startChat(DEFAULT_AGENT, text, attachments)
            }
          />
          <LiveRunsSection finished={false} />
          <RecentChats agents={agents} />
        </div>
      </ScrollArea>
    </div>
  )
}

/** Recent: pinned chats, then the newest, ten in all; the rest folded */
function RecentChats({ agents }: { agents: readonly AgentInfo[] }) {
  const { t } = useTranslation()
  const chats = useChats().data ?? []
  const pinned = useChatPinsStore(state => state.pinned)
  const { recent, older } = recentChats(chats, pinned)
  if (chats.length === 0) return null

  return (
    <section aria-labelledby="recent-chats-heading">
      <h2
        id="recent-chats-heading"
        className="px-1 pb-1 text-xs font-medium text-muted-foreground"
      >
        {t('nandiHome.recent')}
      </h2>
      <ul aria-label={t('nandiHome.recent')} className="flex flex-col gap-1">
        {recent.map(chat => (
          <li key={chat.id}>
            <ChatRow
              chat={chat}
              agents={agents}
              pinned={pinned.includes(chat.id)}
            />
          </li>
        ))}
      </ul>
      {older.length > 0 && (
        <Collapsible className="mt-2">
          <CollapsibleTrigger className="group flex items-center gap-1 px-1 text-xs text-muted-foreground hover:text-foreground">
            <ChevronRight
              aria-hidden
              className="size-3.5 transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180"
            />
            {t('nandiHome.older', { count: older.length })}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <ul className="mt-1 flex flex-col gap-1">
              {older.map(chat => (
                <li key={chat.id}>
                  <ChatRow chat={chat} agents={agents} pinned={false} />
                </li>
              ))}
            </ul>
          </CollapsibleContent>
        </Collapsible>
      )}
    </section>
  )
}

function ChatRow({
  chat,
  agents,
  pinned,
}: {
  chat: ChatSummary
  agents: readonly AgentInfo[]
  pinned: boolean
}) {
  const { t, i18n } = useTranslation()
  const agent =
    agents.find(a => a.id === chat.agentId) ??
    displayAgent(chat.agentId, chat.agentId)
  const label = labelText(chatLabel(chat, agents), t)

  return (
    <div className="flex items-center gap-1 rounded-md border hover:bg-accent">
      <button
        type="button"
        onClick={() =>
          useNavigationStore
            .getState()
            .navigate({ kind: 'nandi', conversationId: chat.id })
        }
        className="flex min-w-0 flex-1 items-center gap-3 px-3 py-2 text-start"
      >
        <AgentAvatar agent={agent} className="size-6 shrink-0" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium">{label}</span>
          <span className="truncate text-xs text-muted-foreground">
            {chat.agentId === DEFAULT_AGENT
              ? chat.lastMessage
              : [agent.name, chat.lastMessage].filter(Boolean).join(' · ')}
          </span>
        </span>
        <span className="shrink-0 text-xs text-muted-foreground">
          {formatRelativeTime(chat.updatedAt, i18n.language)}
        </span>
      </button>
      <Button
        size="icon"
        variant="ghost"
        className="me-1 size-7 shrink-0"
        aria-pressed={pinned}
        aria-label={t(pinned ? 'nandiHome.unpin' : 'nandiHome.pin', {
          name: label,
        })}
        title={t(pinned ? 'nandiHome.unpin' : 'nandiHome.pin', { name: label })}
        onClick={() => useChatPinsStore.getState().toggle(chat.id)}
      >
        {pinned ? <PinOff /> : <Pin />}
      </Button>
    </div>
  )
}

/** One chat with an agent: its runs as they stream, Stop, and a composer */
function ChatView({ conversationId }: { conversationId: string }) {
  const { t } = useTranslation()
  const chats = useChats()
  const agents = useAgents().data ?? []
  const conversation = useConversation(conversationId)
  const agentId = chatAgentId(conversationId, chats.data ?? [])

  useEffect(() => {
    if (!conversation) void loadConversation(conversationId)
  }, [conversationId, conversation])

  if (!agentId) {
    if (chats.isPending) {
      return (
        <div className="flex h-full items-center justify-center">
          <Spinner />
        </div>
      )
    }
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>{t('liveWork.gone')}</EmptyTitle>
        </EmptyHeader>
      </Empty>
    )
  }

  const summary = chats.data?.find(chat => chat.id === conversationId)
  const agent =
    agents.find(a => a.id === agentId) ?? displayAgent(agentId, agentId)
  const title = summary
    ? labelText(chatLabel(summary, agents), t)
    : t('nandiHome.untitled')
  const current = conversation?.currentRun ?? null
  const messages = conversation?.messages ?? []

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label={t('views.back')}
              onClick={() =>
                useNavigationStore.getState().navigate({ kind: 'nandi' })
              }
            >
              <ArrowLeft className="rtl:rotate-180" />
            </Button>
            <AgentAvatar agent={agent} className="size-8" />
          </div>
        }
        title={title}
        titleAdornment={current && <LiveBadge status={current.status} />}
        subtitle={agent.name}
        actions={current && <LiveStop runId={current.id} />}
      />
      {conversation ? (
        <MessageList
          messages={messages}
          assistant={agent}
          working={isWorking(current, messages)}
          quietSteps
          onDecide={(approvalId, decision) =>
            decideApproval(agentId, approvalId, decision)
          }
          empty={
            <Empty className="h-full">
              <EmptyHeader>
                <MessageSquare className="text-muted-foreground" />
                <EmptyTitle>
                  {t('nandiHome.emptyChat', { name: agent.name })}
                </EmptyTitle>
              </EmptyHeader>
            </Empty>
          }
        />
      ) : (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      )}
      <div className="shrink-0 border-t p-3">
        <Composer
          placeholder={t('liveWork.reply.placeholder', { name: agent.name })}
          agentId={agentId}
          onSend={(text, attachments) =>
            void sendChatMessage(conversationId, text, attachments)
          }
        />
      </div>
    </div>
  )
}
