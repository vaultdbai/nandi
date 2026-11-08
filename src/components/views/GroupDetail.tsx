/**
 * One group, as its members see it: the policy (read-only, with whether
 * its signature checks out; admins edit it, which signs the next
 * version), my access level for the group, members with roles, invites,
 * and Leave.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ArrowLeft,
  Copy,
  LogOut,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { ConfirmAction } from '@/components/network/ConfirmAction'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { toastGroupError } from '@/components/network/group-toast'
import {
  ACCESS_LEVELS,
  accessKey,
  expiresAt,
  isAdmin,
  isLastAdmin,
  policyIsStricter,
  sortMembers,
} from '@/components/network/groups-model'
import {
  useGroup,
  useInviteToGroup,
  useLeaveGroup,
  useRemoveGroupMember,
  useRevokeGroupInvite,
  useSetGroupAccess,
  useSetGroupPolicy,
  useSetGroupRole,
} from '@/hooks/useGroups'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import type { AccessLevel, GroupView } from '@/lib/bindings'
import { isRelayUnavailable } from '@/lib/friendly-error'

export function GroupDetail({
  groupId,
  onBack,
}: {
  groupId: string
  onBack: () => void
}) {
  const { t } = useTranslation()
  const group = useGroup(groupId)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="shrink-0 border-b px-4 py-2">
        <Button size="sm" variant="ghost" onClick={onBack}>
          <ArrowLeft className="rtl:rotate-180" />
          {t('groups.back')}
        </Button>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 p-6">
          {group.isPending && <Skeleton className="h-48" />}
          {group.error &&
            (isRelayUnavailable(group.error) ? (
              <RelayConnectNotice />
            ) : (
              <p className="text-sm text-destructive">
                {t('groups.loadError', { error: group.error.message })}
              </p>
            ))}
          {group.data && <GroupBody view={group.data} onLeft={onBack} />}
        </div>
      </ScrollArea>
    </div>
  )
}

function GroupBody({ view, onLeft }: { view: GroupView; onLeft: () => void }) {
  const { t } = useTranslation()
  const leave = useLeaveGroup()
  const { summary } = view.group
  const onLeave = () =>
    leave.mutate(summary.id, {
      onSuccess: () => {
        toast.success(t('groups.left', { name: summary.name }))
        onLeft()
      },
      onError: error => toastGroupError(t('groups.leaveFailed'), error),
    })

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="flex flex-wrap items-center gap-2 text-lg font-semibold">
            <span className="truncate">{summary.name}</span>
            <Badge variant={isAdmin(view) ? 'default' : 'secondary'}>
              {t(isAdmin(view) ? 'groups.role.admin' : 'groups.role.member')}
            </Badge>
          </h2>
          <p className="text-sm text-muted-foreground">
            {summary.description || t('groups.noDescription')}
          </p>
        </div>
        <ConfirmAction
          trigger={
            <Button variant="outline" disabled={leave.isPending}>
              <LogOut />
              {t('groups.leave')}
            </Button>
          }
          title={t('groups.leaveTitle', { name: summary.name })}
          description={t('groups.leaveDescription')}
          confirmLabel={t('groups.leave')}
          onConfirm={onLeave}
        />
      </div>
      <AccessCard view={view} />
      <PolicyCard view={view} />
      <MembersCard view={view} />
      <InvitesCard view={view} />
    </>
  )
}

function AccessCard({ view }: { view: GroupView }) {
  const { t } = useTranslation()
  const setAccess = useSetGroupAccess()
  const groupId = view.group.summary.id
  const onChange = (level: string) =>
    setAccess.mutate(
      { groupId, level: level as AccessLevel },
      {
        onSuccess: () => toast.success(t('groups.access.saved')),
        onError: error => toastGroupError(t('groups.access.failed'), error),
      }
    )
  return (
    <Card data-testid="group-access">
      <CardHeader>
        <CardTitle>{t('groups.access.title')}</CardTitle>
        <CardDescription>{t('groups.access.description')}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <RadioGroup
          value={view.myAccess}
          onValueChange={onChange}
          disabled={setAccess.isPending}
          aria-label={t('groups.access.title')}
        >
          {ACCESS_LEVELS.map(level => (
            <div key={level} className="flex items-start gap-3">
              <RadioGroupItem
                value={level}
                id={`access-${level}`}
                className="mt-0.5"
              />
              <Label
                htmlFor={`access-${level}`}
                className="flex flex-col items-start gap-0.5"
              >
                <span>{t(`groups.access.${accessKey(level)}`)}</span>
                <span className="text-xs font-normal text-muted-foreground">
                  {t(`groups.access.${accessKey(level)}Hint`)}
                </span>
              </Label>
            </div>
          ))}
        </RadioGroup>
        {policyIsStricter(view.policy.approval, view.myAccess) && (
          <p
            className="text-sm text-muted-foreground"
            data-testid="group-policy-stricter"
          >
            {t('groups.access.policyStricter')}
          </p>
        )}
      </CardContent>
    </Card>
  )
}

function PolicyCard({ view }: { view: GroupView }) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<string | null>(null)
  const save = useSetGroupPolicy()
  const { policy } = view
  const author = policy.authorName || policy.author
  const onSave = () => {
    if (draft === null) return
    save.mutate(
      { groupId: view.group.summary.id, policy: draft },
      {
        onSuccess: () => {
          toast.success(t('groups.policySaved'))
          setDraft(null)
        },
        onError: error => toastGroupError(t('groups.policyFailed'), error),
      }
    )
  }
  return (
    <Card data-testid="group-policy">
      <CardHeader>
        <CardTitle>{t('groups.policy')}</CardTitle>
        <CardDescription>{t('groups.policyIntro')}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {policy.verified ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <ShieldCheck className="size-4 shrink-0" aria-hidden />
            {t('groups.verified', { author, version: policy.version })}
          </p>
        ) : (
          <Alert variant="destructive" data-testid="group-policy-unverified">
            <ShieldAlert />
            <AlertTitle>{t('groups.unverified')}</AlertTitle>
            <AlertDescription>{t('groups.unverifiedHint')}</AlertDescription>
          </Alert>
        )}
        {draft === null ? (
          <pre
            className="max-h-80 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs whitespace-pre-wrap"
            dir="ltr"
          >
            {view.group.policy.text}
          </pre>
        ) : (
          <Textarea
            value={draft}
            rows={14}
            dir="ltr"
            className="font-mono text-xs"
            aria-label={t('groups.policy')}
            onChange={event => setDraft(event.target.value)}
          />
        )}
        {isAdmin(view) && (
          <div className="flex flex-wrap gap-2">
            {draft === null ? (
              <Button
                variant="outline"
                onClick={() => setDraft(view.group.policy.text)}
              >
                {t('groups.editPolicy')}
              </Button>
            ) : (
              <>
                <Button disabled={save.isPending} onClick={onSave}>
                  {save.isPending && <Spinner />}
                  {t('groups.savePolicy')}
                </Button>
                <Button variant="ghost" onClick={() => setDraft(null)}>
                  {t('network.cancel')}
                </Button>
              </>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function MembersCard({ view }: { view: GroupView }) {
  const { t } = useTranslation()
  const setRole = useSetGroupRole()
  const remove = useRemoveGroupMember()
  const groupId = view.group.summary.id
  const admin = isAdmin(view)
  return (
    <Card data-testid="group-members">
      <CardHeader>
        <CardTitle>
          {t('groups.memberCount', { count: view.group.members.length })}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col divide-y">
        {sortMembers(view.group.members).map(member => {
          const name = member.displayName || member.instanceId
          const me = member.instanceId === view.me
          const last = isLastAdmin(view, member.instanceId)
          return (
            <div
              key={member.instanceId}
              className="flex flex-wrap items-center justify-between gap-2 py-2"
            >
              <div className="min-w-0">
                <p className="truncate text-sm">
                  {name}
                  {me && (
                    <span className="text-muted-foreground">
                      {' '}
                      · {t('groups.you')}
                    </span>
                  )}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t(
                    member.role === 'admin'
                      ? 'groups.role.admin'
                      : 'groups.role.member'
                  )}
                </p>
              </div>
              {admin && !me && (
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={setRole.isPending || last}
                    onClick={() =>
                      setRole.mutate(
                        {
                          groupId,
                          instanceId: member.instanceId,
                          role: member.role === 'admin' ? 'member' : 'admin',
                        },
                        {
                          onError: error =>
                            toastGroupError(t('groups.roleFailed'), error),
                        }
                      )
                    }
                  >
                    {t(
                      member.role === 'admin'
                        ? 'groups.makeMember'
                        : 'groups.makeAdmin'
                    )}
                  </Button>
                  <ConfirmAction
                    trigger={
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={remove.isPending}
                      >
                        <Trash2 />
                        {t('groups.remove')}
                      </Button>
                    }
                    title={t('groups.removeTitle', { name })}
                    description={t('groups.removeDescription')}
                    confirmLabel={t('groups.remove')}
                    onConfirm={() =>
                      remove.mutate(
                        { groupId, instanceId: member.instanceId },
                        {
                          onError: error =>
                            toastGroupError(t('groups.removeFailed'), error),
                        }
                      )
                    }
                  />
                </div>
              )}
            </div>
          )
        })}
      </CardContent>
    </Card>
  )
}

function InvitesCard({ view }: { view: GroupView }) {
  const { t, i18n } = useTranslation()
  const snapshot = useNetworkSnapshot()
  const invite = useInviteToGroup()
  const revoke = useRevokeGroupInvite()
  const [person, setPerson] = useState('')
  const [uses, setUses] = useState(1)
  const [days, setDays] = useState(7)
  const groupId = view.group.summary.id
  const inGroup = new Set(view.group.members.map(m => m.instanceId))
  const people = (snapshot.data?.members ?? []).filter(
    m => !inGroup.has(m.instanceId)
  )

  const copy = (code: string) =>
    void navigator.clipboard?.writeText(code).then(
      () => toast.success(t('groups.codeCopied')),
      () => toast.error(t('groups.copyFailed'))
    )

  const onInvite = (instanceId: string | null) =>
    invite.mutate(
      {
        groupId,
        invite: {
          instanceId,
          maxUses: instanceId ? 1 : Math.max(1, uses),
          expiresAt: expiresAt(days),
        },
      },
      {
        onSuccess: created => {
          setPerson('')
          if (instanceId) {
            toast.success(t('groups.invitedPerson'))
          } else {
            copy(created.code)
          }
        },
        onError: error => toastGroupError(t('groups.inviteFailed'), error),
      }
    )

  return (
    <Card data-testid="group-invites-card">
      <CardHeader>
        <CardTitle>{t('groups.invites')}</CardTitle>
        <CardDescription>{t('groups.invitesHint')}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-wrap items-end gap-2">
          <Field className="min-w-48 flex-1">
            <FieldLabel htmlFor="invite-person">
              {t('groups.invitePerson')}
            </FieldLabel>
            <NativeSelect
              id="invite-person"
              value={person}
              onChange={event => setPerson(event.target.value)}
            >
              <NativeSelectOption value="">
                {t('groups.choosePerson')}
              </NativeSelectOption>
              {people.map(member => (
                <NativeSelectOption
                  key={member.instanceId}
                  value={member.instanceId}
                >
                  {member.displayName || member.instanceId}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
          <Button
            disabled={!person || invite.isPending}
            onClick={() => onInvite(person)}
          >
            {t('groups.invite')}
          </Button>
        </div>
        <div className="flex flex-wrap items-end gap-2">
          <Field className="w-28">
            <FieldLabel htmlFor="invite-uses">{t('groups.uses')}</FieldLabel>
            <Input
              id="invite-uses"
              type="number"
              min={1}
              max={500}
              value={uses}
              onChange={event => setUses(Number(event.target.value) || 1)}
            />
          </Field>
          <Field className="w-28">
            <FieldLabel htmlFor="invite-days">{t('groups.days')}</FieldLabel>
            <Input
              id="invite-days"
              type="number"
              min={0}
              value={days}
              onChange={event => setDays(Number(event.target.value) || 0)}
            />
          </Field>
          <Button
            variant="outline"
            disabled={invite.isPending}
            onClick={() => onInvite(null)}
          >
            {invite.isPending && <Spinner />}
            {t('groups.createCode')}
          </Button>
        </div>
        {view.group.invites.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('groups.noInvites')}
          </p>
        ) : (
          <div className="flex flex-col divide-y">
            {view.group.invites.map(open => (
              <div
                key={open.code}
                className="flex flex-wrap items-center justify-between gap-2 py-2"
              >
                <div className="min-w-0">
                  <p className="truncate font-mono text-sm" dir="ltr">
                    {open.code}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {open.instanceId
                      ? t('groups.inviteFor', {
                          name:
                            snapshot.data?.members.find(
                              m => m.instanceId === open.instanceId
                            )?.displayName ?? open.instanceId,
                        })
                      : t('groups.usesLeft', { count: open.usesLeft })}
                    {open.expiresAt != null &&
                      ` · ${t('groups.expires', { when: new Date(open.expiresAt).toLocaleDateString(i18n.language) })}`}
                  </p>
                </div>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => copy(open.code)}
                  >
                    <Copy />
                    {t('groups.copy')}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={revoke.isPending}
                    onClick={() =>
                      revoke.mutate(open.code, {
                        onError: error =>
                          toastGroupError(t('groups.revokeFailed'), error),
                      })
                    }
                  >
                    {t('groups.revoke')}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
