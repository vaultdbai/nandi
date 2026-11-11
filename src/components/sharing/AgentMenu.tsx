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
  Users,
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
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  useAgentSharing,
  useSetRelaySharing,
  useToggleTeamShare,
} from '@/hooks/useAgentSharing'
import { rawError } from '@/lib/friendly-error'
import { openAgentInStudio } from '@/lib/ask'
import { cn } from '@/lib/utils'
import {
  MANAGE_TEAMS,
  useNavigationStore,
  type View,
} from '@/store/navigation-store'
import { toastShared } from '@/components/team/share-toast'
import type { AgentInfo } from '@/types/agent'

/** Sharing out is outward: it asks first (README principle 7) */
type PendingShare =
  { kind: 'team'; id: string; name: string } | { kind: 'relay' }

/**
 * An agent's menu, from its always-visible "⋯" button (or by right-clicking
 * its row): open it, chat, runs, Edit in Studio, sharing, and delete.
 * Sharing goes only to my teams (their private registries) or the relay,
 * never to the OS share sheet.
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
  const [pending, setPending] = useState<PendingShare | null>(null)

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
            onRequestShare={setPending}
          />
        </DropdownMenuContent>
      </DropdownMenu>
      {pending && (
        <ConfirmShare
          agent={agent}
          share={pending}
          onClose={() => setPending(null)}
        />
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
  onRequestShare: (share: PendingShare) => void
}) {
  const { t } = useTranslation()
  const { teams, teamsLoading, sharing } = useAgentSharing(agent.id)
  const toggleTeam = useToggleTeamShare(agent.id)
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
      <DropdownMenuSub>
        <DropdownMenuSubTrigger disabled={!mine} className="gap-2">
          <Users className="size-4 text-muted-foreground" />
          {t('agentMenu.shareWithTeam')}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent className="w-52">
          {teamsLoading ? (
            <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
              {t('agentMenu.loadingTeams')}
            </DropdownMenuLabel>
          ) : teams.length === 0 ? (
            <DropdownMenuItem
              onSelect={() =>
                navigate({ kind: 'network', tab: 'people', team: MANAGE_TEAMS })
              }
            >
              {t('agentMenu.noTeams')}
            </DropdownMenuItem>
          ) : (
            teams.map(team => {
              const shared = sharing.teamIds.includes(team.id)
              return (
                <DropdownMenuCheckboxItem
                  key={team.id}
                  checked={shared}
                  disabled={toggleTeam.isPending}
                  onCheckedChange={next => {
                    if (next === true) {
                      onRequestShare({
                        kind: 'team',
                        id: team.id,
                        name: team.name,
                      })
                      return
                    }
                    toggleTeam.mutate(
                      { teamId: team.id, share: false },
                      {
                        onSuccess: () =>
                          toast.success(
                            t('agentMenu.unsharedFromTeam', {
                              name: agent.name,
                              team: team.name,
                            })
                          ),
                        onError: error =>
                          toast.error(t('teams.error.unshare'), {
                            description: rawError(error),
                          }),
                      }
                    )
                  }}
                >
                  {team.name}
                </DropdownMenuCheckboxItem>
              )
            })
          )}
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      <DropdownMenuCheckboxItem
        checked={sharing.relay}
        disabled={!mine || setRelay.isPending}
        onCheckedChange={next => {
          if (next === true) {
            onRequestShare({ kind: 'relay' })
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
      {onDelete && mine && !agent.builtin && (
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

/** "Share Writer with Research?" / "with everyone on your relay?" */
function ConfirmShare({
  agent,
  share,
  onClose,
}: {
  agent: AgentInfo
  share: PendingShare
  onClose: () => void
}) {
  const { t } = useTranslation()
  const toggleTeam = useToggleTeamShare(agent.id)
  const setRelay = useSetRelaySharing(agent.id)

  const confirm = () => {
    if (share.kind === 'team') {
      toggleTeam.mutate(
        { teamId: share.id, share: true },
        {
          onSuccess: result => toastShared(result, agent.name, share.name),
          onError: error =>
            toast.error(t('teams.error.share'), {
              description: rawError(error),
            }),
        }
      )
    } else {
      setRelay.mutate(true, {
        onSuccess: () =>
          toast.success(t('agentMenu.relayOn', { name: agent.name })),
        onError: error =>
          toast.error(t('agentMenu.relayError'), {
            description: rawError(error),
          }),
      })
    }
    onClose()
  }

  return (
    <AlertDialog open onOpenChange={open => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {share.kind === 'team'
              ? t('agentMenu.confirmShare.teamTitle', {
                  name: agent.name,
                  team: share.name,
                })
              : t('agentMenu.confirmShare.relayTitle', { name: agent.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {share.kind === 'team'
              ? t('agentMenu.confirmShare.teamDescription', {
                  team: share.name,
                })
              : t('agentMenu.confirmShare.relayDescription')}
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
