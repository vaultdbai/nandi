import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  ExternalLink,
  ListFilter,
  MessageSquare,
  MoreHorizontal,
  PencilRuler,
  RadioTower,
  Settings2,
  Trash2,
} from 'lucide-react'
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
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useAgentSharing, useSetRelaySharing } from '@/hooks/useAgentSharing'
import { rawError } from '@/lib/friendly-error'
import { openAgentInStudio } from '@/lib/ask'
import { isDefaultAgent } from '@/lib/agent-identity'
import { cn } from '@/lib/utils'
import { useNavigationStore, type View } from '@/store/navigation-store'
import type { AgentInfo } from '@/types/agent'

/**
 * An agent's menu, from its always-visible "⋯" button (or by right-clicking
 * its row): open it, chat, runs, Edit in Studio, sharing, and delete.
 * Sharing goes only to the relay, never to the OS share sheet. Sharing out
 * is outward, so it asks first (README principle 7).
 */
export function AgentMenu({
  agent,
  open,
  onOpenChange,
  onOpenSharing,
  onDelete,
  className,
}: {
  agent: AgentInfo
  open: boolean
  onOpenChange: (open: boolean) => void
  onOpenSharing: () => void
  /** Offers Delete… (the caller shows the confirmation) */
  onDelete?: () => void
  className?: string
}) {
  const { t } = useTranslation()
  const [confirming, setConfirming] = useState(false)

  return (
    <>
      <DropdownMenu open={open} onOpenChange={onOpenChange}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={t('agentMenu.label', { name: agent.name })}
            className={cn(
              'rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none',
              className
            )}
          >
            <MoreHorizontal aria-hidden className="size-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          {/* Mounted only while open, so closed rows run no sharing queries */}
          <AgentMenuItems
            agent={agent}
            onOpenSharing={onOpenSharing}
            onDelete={onDelete}
            onRequestShare={() => setConfirming(true)}
          />
        </DropdownMenuContent>
      </DropdownMenu>
      {confirming && (
        <ConfirmShare agent={agent} onClose={() => setConfirming(false)} />
      )}
    </>
  )
}

function AgentMenuItems({
  agent,
  onOpenSharing,
  onDelete,
  onRequestShare,
}: {
  agent: AgentInfo
  onOpenSharing: () => void
  onDelete?: () => void
  onRequestShare: () => void
}) {
  const { t } = useTranslation()
  const { sharing } = useAgentSharing(agent.id)
  const setRelay = useSetRelaySharing(agent.id)
  // Marketplace installs are someone else's agent: only they can share it
  const mine = agent.origin.kind === 'local'
  const navigate = (view: View) => useNavigationStore.getState().navigate(view)

  return (
    <>
      <DropdownMenuItem
        onSelect={() => navigate({ kind: 'agent', agentId: agent.id })}
      >
        <ExternalLink />
        {t('agentMenu.open')}
      </DropdownMenuItem>
      <DropdownMenuItem
        onSelect={() =>
          navigate({ kind: 'agent', agentId: agent.id, tab: 'chat' })
        }
      >
        <MessageSquare />
        {t('agentMenu.chat')}
      </DropdownMenuItem>
      <DropdownMenuItem
        onSelect={() =>
          navigate({ kind: 'agent', agentId: agent.id, tab: 'runs' })
        }
      >
        <ListFilter />
        {t('agentMenu.runs')}
      </DropdownMenuItem>
      {mine && (
        <DropdownMenuItem onSelect={() => openAgentInStudio(agent.id)}>
          <PencilRuler />
          {t('agentPage.editInStudio')}
        </DropdownMenuItem>
      )}
      <DropdownMenuSeparator />
      {!mine && (
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
          {t('agentMenu.notYours')}
        </DropdownMenuLabel>
      )}
      <DropdownMenuCheckboxItem
        checked={sharing.relay}
        disabled={!mine || setRelay.isPending}
        onCheckedChange={next => {
          if (next === true) {
            onRequestShare()
            return
          }
          setRelay.mutate(false, {
            onSuccess: () =>
              toast.success(t('agentMenu.relayOff', { name: agent.name })),
            onError: error =>
              toast.error(t('agentMenu.relayError'), {
                description: rawError(error),
              }),
          })
        }}
      >
        <RadioTower />
        {t('agentMenu.relay')}
      </DropdownMenuCheckboxItem>
      <DropdownMenuItem onSelect={onOpenSharing}>
        <Settings2 />
        {t('agentMenu.sharingSettings')}
      </DropdownMenuItem>
      {onDelete && mine && !isDefaultAgent(agent) && (
        <>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onSelect={onDelete}>
            <Trash2 />
            {t('agentPage.delete.action')}
          </DropdownMenuItem>
        </>
      )}
    </>
  )
}

/** "Share Writer with everyone on your relay?" */
function ConfirmShare({
  agent,
  onClose,
}: {
  agent: AgentInfo
  onClose: () => void
}) {
  const { t } = useTranslation()
  const setRelay = useSetRelaySharing(agent.id)

  const confirm = () => {
    setRelay.mutate(true, {
      onSuccess: () =>
        toast.success(t('agentMenu.relayOn', { name: agent.name })),
      onError: error =>
        toast.error(t('agentMenu.relayError'), {
          description: rawError(error),
        }),
    })
    onClose()
  }

  return (
    <AlertDialog open onOpenChange={open => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('agentMenu.confirmShare.relayTitle', { name: agent.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t('agentMenu.confirmShare.relayDescription')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>
            {t('agentMenu.confirmShare.cancel')}
          </AlertDialogCancel>
          <AlertDialogAction onClick={confirm}>
            {t('agentMenu.confirmShare.confirm')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
