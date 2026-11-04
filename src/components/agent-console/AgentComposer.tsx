import { useTranslation } from 'react-i18next'
import { AskBox } from '@/components/ask/AskBox'
import { LlmKeyNotice } from '@/components/layout/LlmKeyNotice'
import { sendAgentMessage } from '@/lib/agent-actions'
import { askContextForView, openEditInStudio } from '@/lib/ask'
import { useAgent, useCurrentRun } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * The agent page's message box: Chat messages the agent (queued while a
 * run is going), Ask asks about it. It used to be the bottom bar docked on
 * every page; v1 keeps it only where you talk to an agent.
 */
export function AgentComposer({ agentId }: { agentId: string }) {
  const { t } = useTranslation()
  const agent = useAgent(agentId)?.agent
  const run = useCurrentRun(agentId)
  const view = useNavigationStore(state => state.current)

  return (
    <div className="shrink-0">
      <LlmKeyNotice />
      <div className="border-t bg-background px-4 pt-3 pb-3">
        <AskBox
          className="mx-auto max-w-3xl"
          context={askContextForView(view)}
          placeholder={t('ask.placeholder.agent', {
            agentName: agent?.name ?? agentId,
          })}
          onApplyEdit={openEditInStudio}
          chat={{
            placeholder: t(
              // Mid-run, a message waits in the agent's queue
              run
                ? 'composer.placeholder.agentBusy'
                : 'composer.placeholder.agent',
              { agentName: agent?.name ?? agentId }
            ),
            disabled: !agent,
            onSend: (text, attachments) =>
              void sendAgentMessage(agentId, text, attachments),
          }}
        />
      </div>
    </div>
  )
}
