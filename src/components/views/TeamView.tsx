/**
 * Teams: a shared git registry. My teams, create one from a registry URL,
 * members, shared agents and packs, sync, and share an agent; the roster
 * (each member's gateway and delegations with them) and recent activity.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Bot,
  LogOut,
  Package,
  Plus,
  RefreshCw,
  Share2,
  Users,
} from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { GithubSignInPanel } from '@/components/github/GithubSignInPanel'
import { isGithubUrl } from '@/components/github/github-url'
import { TeamActivityFeed } from '@/components/team/TeamActivityFeed'
import { TeamRosterSection } from '@/components/team/TeamRosterSection'
import { toastShared } from '@/components/team/share-toast'
import { useAgents } from '@/hooks/useAgents'
import {
  useShareAgent,
  useTeam,
  useTeamAction,
  useTeams,
  type TeamAction,
} from '@/hooks/useTeams'
import type { AgentCard, AgentInfo, Team, TeamMember } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { ViewHeader } from './ViewHeader'

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error)

/** Manage teams (People › Team › Manage teams…), opening `initialTeam` */
export function TeamView({ initialTeam }: { initialTeam?: string } = {}) {
  const { t } = useTranslation()
  const teams = useTeams()
  const [selected, setSelected] = useState<string | null>(initialTeam ?? null)
  const [creating, setCreating] = useState(false)

  const list = teams.data ?? []
  // The chosen team, else the first one
  const current =
    list.find(team => team.id === selected)?.id ?? list[0]?.id ?? null

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('teams.title')}
        subtitle={t('teams.subtitle')}
        leading={<Users className="size-4 text-muted-foreground" />}
        actions={
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus />
            {t('teams.create')}
          </Button>
        }
      />
      <div className="flex min-h-0 flex-1">
        {teams.isLoading && (
          <div className="grid w-full gap-3 p-6">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-40 w-full" />
          </div>
        )}
        {teams.error && (
          <p className="p-6 text-sm text-destructive">
            {errorText(teams.error)}
          </p>
        )}
        {!teams.isLoading && !teams.error && list.length === 0 && (
          <Empty className="m-6">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Users />
              </EmptyMedia>
              <EmptyTitle>{t('teams.empty.title')}</EmptyTitle>
              <EmptyDescription>
                {t('teams.empty.description')}
              </EmptyDescription>
            </EmptyHeader>
            <Button onClick={() => setCreating(true)}>
              <Plus />
              {t('teams.create')}
            </Button>
          </Empty>
        )}
        {list.length > 0 && (
          <>
            <TeamList teams={list} current={current} onSelect={setSelected} />
            {current && <TeamDetailPane key={current} teamId={current} />}
          </>
        )}
      </div>
      <CreateTeamDialog open={creating} onOpenChange={setCreating} />
    </div>
  )
}

function TeamList({
  teams,
  current,
  onSelect,
}: {
  teams: Team[]
  current: string | null
  onSelect: (teamId: string) => void
}) {
  const { t } = useTranslation()
  return (
    <nav
      aria-label={t('teams.list.label')}
      className="flex w-56 shrink-0 flex-col gap-1 border-e p-2"
    >
      {teams.map(team => (
        <button
          key={team.id}
          type="button"
          onClick={() => onSelect(team.id)}
          aria-current={team.id === current ? 'page' : undefined}
          className={cn(
            'flex flex-col items-start rounded-md px-3 py-2 text-start text-sm hover:bg-accent',
            team.id === current && 'bg-accent font-medium'
          )}
        >
          <span className="w-full truncate">{team.name}</span>
          <span className="text-xs text-muted-foreground">
            {t('teams.memberCount', { count: team.members.length })}
          </span>
        </button>
      ))}
    </nav>
  )
}

function TeamDetailPane({ teamId }: { teamId: string }) {
  const { t, i18n } = useTranslation()
  const detail = useTeam(teamId)
  const action = useTeamAction()
  const share = useShareAgent()
  const agents = useAgents()
  const [sharing, setSharing] = useState(false)

  const run = (kind: TeamAction) =>
    action.mutate(
      { teamId, action: kind },
      {
        onError: error =>
          toast.error(t(`teams.error.${kind}`), {
            description: errorText(error),
          }),
      }
    )

  const unshare = (card: AgentCard) =>
    share.mutate(
      { teamId, agentId: agentIdOf(card), share: false },
      {
        onError: error =>
          toast.error(t('teams.error.unshare'), {
            description: errorText(error),
          }),
      }
    )

  if (detail.isLoading) {
    return (
      <div className="grid flex-1 gap-3 p-6">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    )
  }
  if (detail.error || !detail.data) {
    return (
      <p className="flex-1 p-6 text-sm text-destructive">
        {errorText(detail.error)}
      </p>
    )
  }

  const { team, agents: shared, packs, lastSyncedAt } = detail.data
  const pending = detail.data.pendingShares ?? []
  const agentName = (agentId: string) =>
    agents.data?.find(agent => agent.id === agentId)?.name ?? agentId
  const busy = action.isPending
  const lastSynced =
    lastSyncedAt === null
      ? t('teams.neverSynced')
      : t('teams.lastSynced', {
          time: new Date(lastSyncedAt).toLocaleString(i18n.language),
        })

  return (
    <ScrollArea className="min-h-0 flex-1">
      <div className="mx-auto grid w-full max-w-4xl gap-4 p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="truncate text-lg font-semibold">{team.name}</h2>
            <p className="font-mono text-xs text-muted-foreground">
              {team.registryRepo}
            </p>
            <p className="text-xs text-muted-foreground">{lastSynced}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => run('sync')}
            >
              {busy && action.variables?.action === 'sync' ? (
                <Spinner />
              ) : (
                <RefreshCw />
              )}
              {t('teams.sync')}
            </Button>
            <Button size="sm" disabled={busy} onClick={() => setSharing(true)}>
              <Share2 />
              {t('teams.share')}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => run('leave')}
            >
              <LogOut />
              {t('teams.leave')}
            </Button>
          </div>
        </div>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Bot className="size-4" />
              {t('teams.agents.title')}
            </CardTitle>
          </CardHeader>
          <CardContent className="grid gap-3">
            {pending.map(share => (
              <div
                key={share.agentId}
                data-testid={`team-pending-${share.agentId}`}
                className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-dashed p-3"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium">
                    {agentName(share.agentId)}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t('teams.pending.hint')}{' '}
                    <code dir="ltr" className="font-mono">
                      {share.branch}
                    </code>
                  </p>
                </div>
                <Badge variant="outline">{t('teams.pending.badge')}</Badge>
              </div>
            ))}
            {shared.length === 0 && pending.length === 0 && (
              <p className="text-sm text-muted-foreground">
                {t('teams.agents.empty')}
              </p>
            )}
            {shared.map(card => (
              <div
                key={card.id}
                data-testid={`team-agent-${agentIdOf(card)}`}
                className="flex items-start justify-between gap-3 rounded-md border p-3"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium">{card.name}</p>
                  {card.description && (
                    <p className="text-sm text-muted-foreground">
                      {card.description}
                    </p>
                  )}
                  <p className="text-xs text-muted-foreground">
                    {t('teams.agents.sharedBy', {
                      name: card.owner.displayName || card.owner.githubHandle,
                    })}
                  </p>
                  {card.capabilities.length > 0 && (
                    <div className="mt-2 flex flex-wrap gap-1.5">
                      {card.capabilities.map(capability => (
                        <Badge key={capability.name} variant="secondary">
                          {capability.name}
                        </Badge>
                      ))}
                    </div>
                  )}
                </div>
                {isMine(card, agents.data) && (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={share.isPending}
                    onClick={() => unshare(card)}
                  >
                    {t('teams.unshare')}
                  </Button>
                )}
              </div>
            ))}
          </CardContent>
        </Card>

        <TeamRosterSection teamId={team.id} />

        <div className="grid gap-4 md:grid-cols-2">
          <MembersCard members={team.members} />
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Package className="size-4" />
                {t('teams.packs.title')}
              </CardTitle>
            </CardHeader>
            <CardContent className="flex flex-wrap gap-1.5">
              {packs.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {t('teams.packs.empty')}
                </p>
              ) : (
                packs.map(pack => (
                  <Badge key={pack} variant="outline">
                    {pack}
                  </Badge>
                ))
              )}
            </CardContent>
          </Card>
        </div>

        <TeamActivityFeed team={team} cards={shared} />
      </div>
      <ShareAgentDialog
        teamId={team.id}
        teamName={team.name}
        open={sharing}
        onOpenChange={setSharing}
      />
    </ScrollArea>
  )
}

/**
 * Whether a shared card is one of my own local agents (only those can be
 * withdrawn). Without a signed-in account, ownership is by local agent ID.
 */
function isMine(card: AgentCard, mine: AgentInfo[] | undefined) {
  const agentId = agentIdOf(card)
  return (mine ?? []).some(
    agent => agent.origin.kind === 'local' && agent.id === agentId
  )
}

/** The agent ID part of a card ID (`<instance>/<agent>`) */
function agentIdOf(card: AgentCard) {
  const prefix = `${card.instanceId}/`
  return card.id.startsWith(prefix) ? card.id.slice(prefix.length) : card.id
}

function MembersCard({ members }: { members: TeamMember[] }) {
  const { t } = useTranslation()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Users className="size-4" />
          {t('teams.members.title')}
        </CardTitle>
        <CardDescription>
          {t('teams.memberCount', { count: members.length })}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ul className="grid gap-2">
          {members.map(member => (
            <li
              key={member.githubHandle}
              className="flex items-center justify-between gap-2 text-sm"
            >
              <span className="flex min-w-0 items-baseline gap-1.5">
                <span className="truncate">
                  {member.displayName || member.githubHandle}
                </span>
                <span
                  className="truncate text-xs text-muted-foreground"
                  dir="ltr"
                >
                  @{member.githubHandle}
                </span>
              </span>
              <Badge
                variant={member.role === 'member' ? 'outline' : 'secondary'}
              >
                {t(`teams.role.${member.role}`)}
              </Badge>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}

function CreateTeamDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const action = useTeamAction()
  const [registryUrl, setRegistryUrl] = useState('')
  const url = registryUrl.trim()

  const submit = () =>
    action.mutate(
      { teamId: url, action: 'create' },
      {
        onSuccess: () => {
          setRegistryUrl('')
          onOpenChange(false)
        },
        onError: error =>
          toast.error(t('teams.error.create'), {
            description: errorText(error),
          }),
      }
    )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('teams.createDialog.title')}</DialogTitle>
          <DialogDescription>
            {t('teams.createDialog.description')}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="team-registry">
            {t('teams.createDialog.registryUrl')}
          </Label>
          <Input
            id="team-registry"
            dir="ltr"
            value={registryUrl}
            placeholder="https://git.example.com/team/nandi-registry.git"
            onChange={event => setRegistryUrl(event.target.value)}
          />
          {isGithubUrl(url) && <GithubSignInPanel showSignOut={false} />}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t('teams.cancel')}
          </Button>
          <Button disabled={!url || action.isPending} onClick={submit}>
            {action.isPending && <Spinner />}
            {t('teams.createDialog.create')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Pick one of my agents and share it with a team (a branch to merge) */
export function ShareAgentDialog({
  teamId,
  teamName,
  open,
  onOpenChange,
}: {
  teamId: string
  teamName: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const agents = useAgents()
  const share = useShareAgent()
  const [agentId, setAgentId] = useState('')

  // Only my own agents: marketplace installs are someone else's
  const mine = (agents.data ?? []).filter(
    agent => agent.origin.kind === 'local'
  )
  const chosen = agentId || mine[0]?.id || ''

  const submit = () => {
    const name = mine.find(agent => agent.id === chosen)?.name ?? chosen
    share.mutate(
      { teamId, agentId: chosen, share: true },
      {
        onSuccess: result => {
          toastShared(result, name, teamName)
          onOpenChange(false)
        },
        onError: error =>
          toast.error(t('teams.error.share'), {
            description: errorText(error),
          }),
      }
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('teams.shareDialog.title')}</DialogTitle>
          <DialogDescription>
            {t('teams.shareDialog.description')}
          </DialogDescription>
        </DialogHeader>
        {mine.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('teams.shareDialog.noAgents')}
          </p>
        ) : (
          <div className="grid gap-2">
            <Label htmlFor="share-agent">{t('teams.shareDialog.agent')}</Label>
            <NativeSelect
              id="share-agent"
              className="w-full"
              value={chosen}
              onChange={event => setAgentId(event.target.value)}
            >
              {mine.map(agent => (
                <NativeSelectOption key={agent.id} value={agent.id}>
                  {agent.name}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t('teams.cancel')}
          </Button>
          <Button disabled={!chosen || share.isPending} onClick={submit}>
            {share.isPending && <Spinner />}
            {t('teams.shareDialog.submit')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
