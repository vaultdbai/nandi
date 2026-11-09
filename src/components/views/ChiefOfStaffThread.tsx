import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { MessageList } from '@/components/agent-console/MessageList'
import { isWorking } from '@/components/agent-console/working'
import { AskBox } from '@/components/ask/AskBox'
import { Spinner } from '@/components/ui/spinner'
import { decideApproval } from '@/lib/agent-actions'
import { CHIEF_OF_STAFF_AGENT_ID, displayAgent } from '@/lib/agent-identity'
import { loadConversation } from '@/lib/agent-sync'
import { askContextForView, openEditInStudio } from '@/lib/ask'
import { useConversation } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * A Chief of Staff thread's conversation (an intake item's, an Inbox
 * item's): its messages, with the approval cards of what it proposes
 * decided in place
 */
export function ThreadConversation({
  conversationId,
}: {
  conversationId: string
}) {
  const { t } = useTranslation()
  const conversation = useConversation(conversationId)
  const assistant = displayAgent(
    CHIEF_OF_STAFF_AGENT_ID,
    t('intakeThread.assistant')
  )

  useEffect(() => {
    if (!conversation) void loadConversation(conversationId)
  }, [conversationId, conversation])

  if (!conversation) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner />
      </div>
    )
  }
  return (
    <MessageList
      messages={conversation.messages}
      assistant={assistant}
      working={isWorking(conversation.currentRun, conversation.messages)}
      onDecide={(approvalId, decision) =>
        decideApproval(CHIEF_OF_STAFF_AGENT_ID, approvalId, decision)
      }
    />
  )
}

/** The ask box of a Chief of Staff thread, in chat mode */
export function ThreadComposer({
  placeholder,
  onSend,
}: {
  placeholder: string
  onSend: (text: string) => void
}) {
  const view = useNavigationStore(state => state.current)
  return (
    <AskBox
      className="mx-auto max-w-3xl"
      context={askContextForView(view)}
      placeholder={placeholder}
      onApplyEdit={openEditInStudio}
      fixedMode="chat"
      chat={{ placeholder, onSend }}
    />
  )
}
