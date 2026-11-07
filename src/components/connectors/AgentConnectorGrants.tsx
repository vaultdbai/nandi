import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  useChangeGrant,
  useConnectorGrants,
  useWorkspaceStatus,
} from '@/hooks/useConnectorHub'
import { useConnectors } from '@/hooks/useConnectors'
import { useEmailStatus } from '@/hooks/useEmail'
import type { GrantAccess } from '@/lib/bindings'
import { agentGrants, grantableConnectors } from './grants-model'

/**
 * On an agent's page: Read and Write for each installed package connector
 * and connected account. Turning one on gives a saved grant; off removes
 * every grant of that access (saved and this computer's).
 */
export function AgentConnectorGrants({ agentId }: { agentId: string }) {
  const { t } = useTranslation()
  const grants = useConnectorGrants()
  const connectors = useConnectors()
  const workspace = useWorkspaceStatus()
  const email = useEmailStatus()
  const change = useChangeGrant()

  const connected = [
    ...(email.data?.providers ?? [])
      .filter(p => p.connected)
      .map(p => p.provider as string),
    ...(workspace.data?.accounts ?? []).filter(a => a.connected).map(a => a.id),
  ]
  const shown = grantableConnectors(connectors.data ?? [], connected)
  if (shown.length === 0 || !grants.data) return null
  const all = grants.data

  const toggle = (connectorId: string, access: GrantAccess, on: boolean) => {
    const onError = (error: Error) =>
      toast.error(t('grants.changeFailed'), { description: error.message })
    if (on) {
      change.mutate(
        { kind: 'grant', agentId, connectorId, access, scope: 'saved' },
        { onError }
      )
      return
    }
    for (const grant of agentGrants(all, agentId, connectorId)[access]) {
      change.mutate({ kind: 'revoke', grant }, { onError })
    }
  }

  return (
    <div className="space-y-2 rounded-md border p-3">
      <div>
        <h3 className="text-sm font-medium">{t('grants.agentTitle')}</h3>
        <p className="text-xs text-muted-foreground">{t('grants.agentHint')}</p>
      </div>
      <ul className="divide-y">
        {shown.map(connector => {
          const mine = agentGrants(all, agentId, connector.id)
          return (
            <li
              key={connector.id}
              className="flex flex-wrap items-center gap-4 py-2 text-sm"
              data-testid={`agent-grant-${connector.id}`}
            >
              <span className="min-w-0 flex-1 font-medium">
                {connector.name}
              </span>
              {(['read', 'write'] as const).map(access => {
                const id = `grant-${agentId}-${connector.id}-${access}`
                const local =
                  mine[access].length > 0 &&
                  mine[access].every(g => g.scope === 'local')
                return (
                  <div key={access} className="flex items-center gap-2">
                    <Switch
                      id={id}
                      checked={mine[access].length > 0}
                      disabled={change.isPending}
                      onCheckedChange={on => toggle(connector.id, access, on)}
                    />
                    <Label htmlFor={id} className="font-normal">
                      {t(`grants.access.${access}`)}
                      {local && (
                        <span className="text-xs text-muted-foreground">
                          {' '}
                          ({t('grants.scope.local')})
                        </span>
                      )}
                    </Label>
                  </div>
                )
              })}
            </li>
          )
        })}
      </ul>
    </div>
  )
}
