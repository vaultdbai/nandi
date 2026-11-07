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
  type GrantRow,
} from '@/components/connectors/grants-model'
import { useAgents } from '@/hooks/useAgents'
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
      <span className="flex-1 font-medium">{row.connectorName}</span>
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
