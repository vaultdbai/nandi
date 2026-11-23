import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { KeyRound } from 'lucide-react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Spinner } from '@/components/ui/spinner'
import {
  groupGrants,
  isMailConnector,
  mailboxChoices,
  mailboxCover,
  mailboxesToSave,
  type GrantRow,
} from '@/components/connectors/grants-model'
import { useAgents } from '@/hooks/useAgents'
import { useEmailStatus } from '@/hooks/useEmail'
import type { GrantInfo } from '@/lib/bindings'
import { useChangeGrant, useConnectorGrants } from '@/hooks/useConnectorHub'
import { useConnectors } from '@/hooks/useConnectors'
import { SettingsGroup, SettingsSectionFrame } from '../SettingsFrame'

/**
 * Settings › Connector access: every grant an agent has on a connector or
 * account, saved or this computer only, each with Remove.
 */
export function ConnectorAccessSection() {
  const { t } = useTranslation()
  const grants = useConnectorGrants()
  const agents = useAgents()
  const connectors = useConnectors()
  const groups = groupGrants(grants.data ?? [], {
    agent: id => agents.data?.find(a => a.id === id)?.name,
    connector: id => connectors.data?.find(c => c.id === id)?.name,
  })

  return (
    <SettingsSectionFrame
      title={t('settings.section.access')}
      description={t('grants.description')}
    >
      {grants.isPending && <Spinner />}
      {grants.error && (
        <p className="text-sm text-destructive">{String(grants.error)}</p>
      )}
      {grants.data && groups.length === 0 && (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <KeyRound />
            </EmptyMedia>
            <EmptyTitle>{t('grants.emptyTitle')}</EmptyTitle>
            <EmptyDescription>{t('grants.emptyBody')}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      {groups.map(group => (
        <SettingsGroup key={group.agentId} title={group.agentName}>
          <ul className="divide-y rounded-md border">
            {group.rows.map(row => (
              <GrantRowView
                key={`${row.grant.connector}-${row.grant.access}-${row.grant.scope}`}
                row={row}
                agentName={group.agentName}
              />
            ))}
          </ul>
        </SettingsGroup>
      ))}
    </SettingsSectionFrame>
  )
}

function GrantRowView({
  row,
  agentName,
}: {
  row: GrantRow
  agentName: string
}) {
  const { t } = useTranslation()
  const change = useChangeGrant()
  const cover = mailboxCover(row.grant)
  const remove = () =>
    change.mutate(
      { kind: 'revoke', grant: row.grant },
      {
        onError: error =>
          toast.error(t('grants.removeFailed'), {
            description: String(error),
          }),
      }
    )

  return (
    <li
      className="flex flex-wrap items-center gap-2 px-3 py-2 text-sm"
      data-testid="grant-row"
    >
      <span className="flex-1 font-medium">
        {row.connectorName}
        {isMailConnector(row.grant.connector) && (
          <span
            className="block text-xs font-normal break-words text-muted-foreground"
            data-testid="grant-mailboxes"
          >
            {cover.kind === 'every'
              ? t('grants.mailboxes.every')
              : t('grants.mailboxes.only', {
                  list: cover.mailboxes.join(', '),
                })}
          </span>
        )}
      </span>
      {isMailConnector(row.grant.connector) && (
        <MailboxChooser grant={row.grant} agentName={agentName} />
      )}
      <Badge variant="outline">{t(`grants.access.${row.grant.access}`)}</Badge>
      <Badge variant="secondary">{t(`grants.scope.${row.grant.scope}`)}</Badge>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button size="sm" variant="ghost" disabled={change.isPending}>
            {t('grants.remove')}
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('grants.removeTitle', {
                agent: agentName,
                connector: row.connectorName,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('grants.removeBody')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('grants.keep')}</AlertDialogCancel>
            <AlertDialogAction onClick={remove}>
              {t('grants.remove')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </li>
  )
}

/**
 * Which mailboxes a mail grant covers: every one (ones connected later
 * too), or only the ticked ones
 */
function MailboxChooser({
  grant,
  agentName,
}: {
  grant: GrantInfo
  agentName: string
}) {
  const { t } = useTranslation()
  const email = useEmailStatus()
  const change = useChangeGrant()
  const [open, setOpen] = useState(false)
  const [every, setEvery] = useState(grant.mailboxes.length === 0)
  const [ticked, setTicked] = useState<string[]>(grant.mailboxes)
  const choices = mailboxChoices(grant, email.data?.mailboxes ?? [])
  const toSave = mailboxesToSave(every, ticked)

  const openWith = (next: boolean) => {
    if (next) {
      setEvery(grant.mailboxes.length === 0)
      setTicked(grant.mailboxes)
    }
    setOpen(next)
  }
  const save = () => {
    if (toSave === null) return
    change.mutate(
      { kind: 'mailboxes', grant, mailboxes: toSave },
      {
        onSuccess: () => setOpen(false),
        onError: error =>
          toast.error(t('grants.mailboxes.failed'), {
            description: String(error),
          }),
      }
    )
  }

  return (
    <Dialog open={open} onOpenChange={openWith}>
      <DialogTrigger asChild>
        <Button size="sm" variant="ghost" disabled={choices.length === 0}>
          {t('grants.mailboxes.choose')}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {t('grants.mailboxes.title', { agent: agentName })}
          </DialogTitle>
          <DialogDescription>
            {t('grants.mailboxes.description')}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Checkbox
              id={`${grant.agent}-${grant.connector}-every`}
              checked={every}
              onCheckedChange={value => setEvery(value === true)}
            />
            <Label
              htmlFor={`${grant.agent}-${grant.connector}-every`}
              className="font-normal"
            >
              {t('grants.mailboxes.everyLater')}
            </Label>
          </div>
          {choices.map(mailbox => (
            <div key={mailbox} className="flex items-center gap-2 ps-6">
              <Checkbox
                id={`${grant.agent}-${grant.connector}-${mailbox}`}
                disabled={every}
                checked={every || ticked.includes(mailbox)}
                onCheckedChange={value =>
                  setTicked(current =>
                    value === true
                      ? [...current, mailbox]
                      : current.filter(m => m !== mailbox)
                  )
                }
              />
              <Label
                htmlFor={`${grant.agent}-${grant.connector}-${mailbox}`}
                className="font-normal"
              >
                {mailbox}
              </Label>
            </div>
          ))}
          {toSave === null && (
            <p className="text-xs text-muted-foreground">
              {t('grants.mailboxes.pickOne')}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>
            {t('grants.keep')}
          </Button>
          <Button disabled={toSave === null || change.isPending} onClick={save}>
            {t('grants.mailboxes.save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
