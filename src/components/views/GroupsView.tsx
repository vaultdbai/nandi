/**
 * Network › Groups: every group on the relay (anyone sees names and
 * descriptions), the invites for me, Join with code and Create group.
 * A group I'm in opens its detail (`GroupDetail`). Without a relay: the
 * calm "connect to your relay" state, never an error.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plus, Ticket, Users } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardDescription,
  CardFooter,
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
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { toastGroupError } from '@/components/network/group-toast'
import { sortGroups } from '@/components/network/groups-model'
import { useCreateGroup, useGroups, useJoinGroup } from '@/hooks/useGroups'
import type { GroupInvite, GroupSummary } from '@/lib/bindings'
import { isRelayUnavailable } from '@/lib/friendly-error'
import { useNavigationStore } from '@/store/navigation-store'
import { GroupDetail } from './GroupDetail'

const openGroup = (group?: string) =>
  useNavigationStore
    .getState()
    .navigate({ kind: 'network', tab: 'groups', group })

export function GroupsView({ groupId }: { groupId?: string }) {
  if (groupId) {
    return <GroupDetail groupId={groupId} onBack={() => openGroup()} />
  }
  return <GroupsList />
}

function GroupsList() {
  const { t } = useTranslation()
  const groups = useGroups()
  const [joining, setJoining] = useState(false)
  const [creating, setCreating] = useState(false)
  // Invites ignored in this session
  const [ignored, setIgnored] = useState<string[]>([])
  const join = useJoinGroup()

  const onAccept = (invite: GroupInvite) =>
    join.mutate(invite.code, {
      onSuccess: view => {
        toast.success(t('groups.joined', { name: view.group.summary.name }))
        openGroup(view.group.summary.id)
      },
      onError: error => toastGroupError(t('groups.joinFailed'), error),
    })

  if (groups.isPending) {
    return (
      <div className="mx-auto grid w-full max-w-5xl gap-4 p-6 md:grid-cols-2">
        {[0, 1].map(i => (
          <Skeleton key={i} className="h-32" />
        ))}
      </div>
    )
  }
  if (groups.error && !isRelayUnavailable(groups.error)) {
    return (
      <p className="p-6 text-sm text-destructive">
        {t('groups.loadError', { error: groups.error.message })}
      </p>
    )
  }
  const data = groups.data
  if (!data?.connected) {
    return (
      <div className="mx-auto w-full max-w-5xl p-6">
        <RelayConnectNotice />
      </div>
    )
  }
  const invites = data.invites.filter(i => !ignored.includes(i.code))
  const list = sortGroups(data.groups)

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 max-w-2xl">
            <h2 className="text-lg font-semibold">{t('groups.title')}</h2>
            <p className="text-sm text-muted-foreground">
              {data.visibility === 'groups'
                ? t('groups.introGroupsRelay')
                : t('groups.intro')}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => setJoining(true)}>
              <Ticket />
              {t('groups.joinWithCode')}
            </Button>
            <Button onClick={() => setCreating(true)}>
              <Plus />
              {t('groups.create')}
            </Button>
          </div>
        </div>

        {invites.length > 0 && (
          <section className="flex flex-col gap-2" data-testid="group-invites">
            <h3 className="text-sm font-medium">{t('groups.invitesForMe')}</h3>
            {invites.map(invite => (
              <div
                key={invite.code}
                className="flex flex-wrap items-center justify-between gap-3 rounded-md border px-3 py-2"
              >
                <p className="min-w-0 text-sm">
                  {t('groups.invitedTo', { name: invite.groupName })}
                </p>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    disabled={join.isPending}
                    onClick={() => onAccept(invite)}
                  >
                    {t('groups.accept')}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setIgnored(list => [...list, invite.code])}
                  >
                    {t('groups.ignore')}
                  </Button>
                </div>
              </div>
            ))}
          </section>
        )}

        {list.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Users aria-hidden />
              </EmptyMedia>
              <EmptyTitle>{t('groups.none')}</EmptyTitle>
              <EmptyDescription>{t('groups.noneHint')}</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            {list.map(group => (
              <GroupCard key={group.id} group={group} />
            ))}
          </div>
        )}
      </div>
      <JoinDialog open={joining} onOpenChange={setJoining} />
      <CreateDialog
        open={creating}
        onOpenChange={setCreating}
        template={data.policyTemplate}
      />
    </ScrollArea>
  )
}

function GroupCard({ group }: { group: GroupSummary }) {
  const { t } = useTranslation()
  const member = group.role !== null
  return (
    <Card data-testid="group-card">
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          <span className="truncate">{group.name}</span>
          {group.role === 'admin' && <Badge>{t('groups.role.admin')}</Badge>}
          {group.role === 'member' && (
            <Badge variant="secondary">{t('groups.role.member')}</Badge>
          )}
        </CardTitle>
        <CardDescription className="line-clamp-3">
          {group.description || t('groups.noDescription')}
        </CardDescription>
      </CardHeader>
      <CardFooter className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs text-muted-foreground">
          {t('groups.memberCount', { count: group.memberCount })}
        </span>
        {member ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => openGroup(group.id)}
          >
            {t('groups.open')}
          </Button>
        ) : (
          <span className="text-xs text-muted-foreground">
            {t('groups.askForInvite')}
          </span>
        )}
      </CardFooter>
    </Card>
  )
}

function JoinDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const [code, setCode] = useState('')
  const join = useJoinGroup()
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    join.mutate(code.trim(), {
      onSuccess: view => {
        toast.success(t('groups.joined', { name: view.group.summary.name }))
        setCode('')
        onOpenChange(false)
        openGroup(view.group.summary.id)
      },
      onError: error => toastGroupError(t('groups.joinFailed'), error),
    })
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>{t('groups.joinWithCode')}</DialogTitle>
            <DialogDescription>{t('groups.joinHint')}</DialogDescription>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="group-code">{t('groups.code')}</FieldLabel>
            <Input
              id="group-code"
              value={code}
              onChange={event => setCode(event.target.value)}
              autoComplete="off"
            />
          </Field>
          <DialogFooter>
            <Button type="submit" disabled={!code.trim() || join.isPending}>
              {join.isPending && <Spinner />}
              {t('groups.join')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function CreateDialog({
  open,
  onOpenChange,
  template,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  template: string
}) {
  const { t } = useTranslation()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [policy, setPolicy] = useState(template)
  const create = useCreateGroup()
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    create.mutate(
      { name, description, policy },
      {
        onSuccess: view => {
          toast.success(t('groups.created', { name: view.group.summary.name }))
          setName('')
          setDescription('')
          setPolicy(template)
          onOpenChange(false)
          openGroup(view.group.summary.id)
        },
        onError: error => toastGroupError(t('groups.createFailed'), error),
      }
    )
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <form onSubmit={submit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>{t('groups.create')}</DialogTitle>
            <DialogDescription>{t('groups.createHint')}</DialogDescription>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="group-name">{t('groups.name')}</FieldLabel>
            <Input
              id="group-name"
              value={name}
              maxLength={80}
              onChange={event => setName(event.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="group-description">
              {t('groups.description')}
            </FieldLabel>
            <Textarea
              id="group-description"
              value={description}
              maxLength={500}
              rows={2}
              onChange={event => setDescription(event.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="group-policy">{t('groups.policy')}</FieldLabel>
            <Textarea
              id="group-policy"
              value={policy}
              rows={14}
              className="font-mono text-xs"
              dir="ltr"
              onChange={event => setPolicy(event.target.value)}
            />
            <FieldDescription>{t('groups.policyHint')}</FieldDescription>
          </Field>
          <DialogFooter>
            <Button type="submit" disabled={!name.trim() || create.isPending}>
              {create.isPending && <Spinner />}
              {t('groups.createAction')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
