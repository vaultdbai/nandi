import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { MessageSquare, Sparkles } from 'lucide-react'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Spinner } from '@/components/ui/spinner'
import { ScrollArea } from '@/components/ui/scroll-area'
import { MessageList } from '@/components/agent-console/MessageList'
import { isWorking } from '@/components/agent-console/working'
import { AskBox } from '@/components/ask/AskBox'
import { AskHistory } from '@/components/ask/AskHistory'
import { ArtifactTabs } from '@/components/studio/ArtifactTabs'
import { AskNewMenu, AskStarters } from '@/components/studio/AskNewMenu'
import { useMascotName } from '@/hooks/useMascot'
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from '@/components/ui/resizable'
import { decideApproval, sendToStudio } from '@/lib/agent-actions'
import { client } from '@/lib/client'
import { displayAgent, STUDIO_AGENT_ID } from '@/lib/agent-identity'
import { loadConversation } from '@/lib/agent-sync'
import { formatRelativeTime } from '@/lib/format-time'
import type { ArtifactRef } from '@/lib/bindings'
import type { StudioConversationSummary } from '@/lib/tauri-bindings'
import { useConversation, useStudioConversations } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { artifactOf, useStudioStore } from '@/store/studio-store'
import { ViewHeader } from './ViewHeader'

/**
 * Studio, called Ask in the app: a split view. Left, the gateway chat: Studio conversations
 * (persisted in SQLite; listed when none is selected) and the ask box,
 * whose approved edits open on the right. Right, the artifact editor:
 * capabilities, agents, workflows, policies, and the tool catalog.
 */
export function StudioView({
  conversationId,
  draft = false,
}: {
  conversationId: string | null
  /** New chat: an empty conversation, created when the first message is sent */
  draft?: boolean
}) {
  const { t } = useTranslation()
  const conversations = useStudioConversations()
  const summary = conversations?.find(c => c.id === conversationId)
  const open = useOpenArtifact()

  return (
    <ResizablePanelGroup direction="horizontal" className="h-full min-h-0">
      <ResizablePanel defaultSize={42} minSize={28}>
        <section
          aria-label={t('studio.chatPane')}
          className="flex h-full min-h-0 flex-col"
        >
          <div className="flex min-h-0 flex-1 flex-col">
            {summary ? (
              <Conversation key={summary.id} summary={summary} />
            ) : draft ? (
              <StudioEmptyState draft />
            ) : conversations && conversations.length > 0 ? (
              <ConversationList conversations={conversations} />
            ) : (
              <StudioEmptyState />
            )}
          </div>
          <div className="shrink-0 border-t p-3">
            <AskBox
              // Fresh draft per conversation
              key={conversationId ?? (draft ? 'new' : 'none')}
              context={{
                currentView: 'studio',
                focusedArtifact: open,
                recentRuns: [],
              }}
              placeholder={t('ask.placeholder.studioEditing')}
              onApplyEdit={preview =>
                useStudioStore.getState().applyEdit(preview)
              }
              chat={{
                placeholder: t('composer.placeholder.studio'),
                onSend: (text, attachments) =>
                  void sendToStudio(text, attachments, conversationId),
              }}
              // One input: with a file open, a change to it comes back as a
              // diff to apply; otherwise Studio answers in the conversation,
              // looking things up with its tools
              fixedMode={open ? 'ask' : 'chat'}
            />
            <AskHistory className="mt-2" />
          </div>
        </section>
      </ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel defaultSize={58} minSize={30}>
        <section
          aria-label={t('studio.artifactPane')}
          className="flex h-full min-h-0 flex-col"
        >
          <ArtifactTabs />
        </section>
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}

/**
 * The artifact open in the current tab, for the ask box's context. Rust
 * hears about it too, so Studio's chat reads "this" as that file.
 */
function useOpenArtifact(): ArtifactRef | null {
  const tab = useStudioStore(state => state.tab)
  const id = useStudioStore(state => state.selected[tab])
  useEffect(() => {
    void client.setStudioOpenArtifact(id ? artifactOf(tab, id) : null)
  }, [tab, id])
  useEffect(() => () => void client.setStudioOpenArtifact(null), [])
  return id ? artifactOf(tab, id) : null
}

/** Every conversation, most recently updated first, plus New chat */
function ConversationList({
  conversations,
}: {
  conversations: StudioConversationSummary[]
}) {
  const { t, i18n } = useTranslation()
  const sorted = [...conversations].sort((a, b) => b.updatedAt - a.updatedAt)

  return (
    <>
      <ViewHeader
        leading={
          <Sparkles aria-hidden className="size-4 text-muted-foreground" />
        }
        title={t('nav.studio')}
        subtitle={t('studio.subtitle')}
        actions={<AskNewMenu />}
      />
      <ScrollArea className="min-h-0 flex-1">
        <ul
          aria-label={t('nav.studioConversations')}
          className="flex flex-col gap-1 p-3"
        >
          {sorted.map(conversation => (
            <li key={conversation.id}>
              <button
                type="button"
                onClick={() =>
                  useNavigationStore.getState().navigate({
                    kind: 'studio',
                    conversationId: conversation.id,
                  })
                }
                className="flex w-full items-start gap-3 rounded-md border px-3 py-2 text-start hover:bg-accent"
              >
                <MessageSquare
                  aria-hidden
                  className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-sm font-medium">
                    {conversation.title || t('studio.untitled')}
                  </span>
                  {conversation.lastMessage && (
                    <span className="truncate text-xs text-muted-foreground">
                      {conversation.lastMessage}
                    </span>
                  )}
                </span>
                <span className="shrink-0 text-xs text-muted-foreground">
                  {formatRelativeTime(conversation.updatedAt, i18n.language)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </ScrollArea>
    </>
  )
}

function Conversation({ summary }: { summary: StudioConversationSummary }) {
  const { t } = useTranslation()
  const conversation = useConversation(summary.id)
  const assistant = displayAgent('studio', useMascotName())

  useEffect(() => {
    if (!conversation) void loadConversation(summary.id)
  }, [summary.id, conversation])

  return (
    <>
      <ViewHeader
        leading={
          <Sparkles aria-hidden className="size-4 text-muted-foreground" />
        }
        title={summary.title || t('studio.untitled')}
        subtitle={t('studio.subtitle')}
        actions={<AskNewMenu />}
      />
      {conversation ? (
        <MessageList
          messages={conversation.messages}
          assistant={assistant}
          working={isWorking(conversation.currentRun, conversation.messages)}
          // Studio's own approvals: saving what it built, starting work
          onDecide={(approvalId, decision) =>
            decideApproval(STUDIO_AGENT_ID, approvalId, decision)
          }
          empty={<StudioEmptyState conversationId={summary.id} />}
        />
      ) : (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      )}
    </>
  )
}

/**
 * Without a conversation, the New menu too (there's no header above). A
 * new chat (`draft`) only invites the first message: sending it creates
 * the conversation.
 */
function StudioEmptyState({
  conversationId = null,
  draft = false,
}: {
  conversationId?: string | null
  draft?: boolean
}) {
  const { t } = useTranslation()

  return (
    <Empty className="h-full">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <MessageSquare />
        </EmptyMedia>
        <EmptyTitle>{t('nav.studio')}</EmptyTitle>
        <EmptyDescription>{t('studio.emptyDescription')}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent className="gap-4">
        {!conversationId && !draft && <AskNewMenu />}
        <AskStarters conversationId={conversationId} />
      </EmptyContent>
    </Empty>
  )
}
