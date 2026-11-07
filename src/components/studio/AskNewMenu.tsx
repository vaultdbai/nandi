import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Bot,
  ChevronDown,
  FileCode,
  MessageSquare,
  Plug,
  Plus,
  Shield,
  Workflow,
} from 'lucide-react'
import { NewAgentDialog } from '@/components/agents/NewAgentDialog'
import { NewConnectorDialog } from '@/components/connectors/NewConnectorDialog'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { openNewChat, sendToStudio } from '@/lib/agent-actions'
import { useStudioStore, type ArtifactTab } from '@/store/studio-store'

/** Opens a right-pane tab with nothing selected, so its New ID field shows */
function openTab(tab: ArtifactTab) {
  const store = useStudioStore.getState()
  store.setTab(tab)
  store.select(tab, null)
}

/**
 * "New ▾" in Ask: a chat, or something to build. Agents get the template
 * dialog, workflows are drafted in the chat, capabilities and policies
 * open their editor tab.
 */
export function AskNewMenu() {
  const { t } = useTranslation()
  const [creatingAgent, setCreatingAgent] = useState(false)
  const [creatingConnector, setCreatingConnector] = useState(false)

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm">
            <Plus />
            {t('studio.newMenu')}
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={openNewChat}>
            <MessageSquare />
            {t('studio.newChatItem')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setCreatingAgent(true)}>
            <Bot />
            {t('studio.tab.agent')}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() =>
              void sendToStudio(t('automations.studioPrompt'), [], null)
            }
          >
            <Workflow />
            {t('studio.tab.workflow')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => openTab('capability')}>
            <FileCode />
            {t('studio.tab.capability')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => openTab('policy')}>
            <Shield />
            {t('studio.tab.policy')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setCreatingConnector(true)}>
            <Plug />
            {t('newConnector.menuItem')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <NewAgentDialog open={creatingAgent} onOpenChange={setCreatingAgent} />
      <NewConnectorDialog
        open={creatingConnector}
        onOpenChange={setCreatingConnector}
      />
    </>
  )
}

const STARTERS = ['waiting', 'today', 'agent', 'workflow'] as const

/** Example asks for an empty chat: two questions, two things to build */
export function AskStarters({
  conversationId,
}: {
  conversationId: string | null
}) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-col items-center gap-2">
      <span className="text-xs text-muted-foreground">
        {t('studio.starters')}
      </span>
      <div className="flex max-w-md flex-wrap justify-center gap-2">
        {STARTERS.map(starter => (
          <Button
            key={starter}
            variant="outline"
            size="sm"
            className="h-auto whitespace-normal py-1.5 text-start"
            onClick={() =>
              void sendToStudio(
                t(`studio.starter.${starter}`),
                [],
                conversationId
              )
            }
          >
            {t(`studio.starter.${starter}`)}
          </Button>
        ))}
      </div>
    </div>
  )
}
