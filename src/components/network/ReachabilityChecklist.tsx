import { useTranslation } from 'react-i18next'
import { Check, CircleAlert, Info, Loader2, Minus } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { useAgents } from '@/hooks/useAgents'
import {
  useAgentDelegation,
  useSetAgentDelegation,
} from '@/hooks/useDelegation'
import { useMakeGatewayReachable } from '@/hooks/useGateway'
import { useReachability } from '@/hooks/useReachability'
import { useTestRelayConnection } from '@/hooks/useRelay'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { Term } from './NetworkHelp'
import {
  doneCount,
  type ReachabilityRow,
  type RowKey,
  type RowStatus,
} from './reachability-model'

const navigate = useNavigationStore.getState().navigate

const ICONS: Record<RowStatus, React.ReactNode> = {
  pass: <Check className="text-success" />,
  fail: <CircleAlert className="text-warning" />,
  warn: <CircleAlert className="text-warning" />,
  info: <Info className="text-muted-foreground" />,
  checking: <Loader2 className="animate-spin text-muted-foreground" />,
  blocked: <Minus className="text-muted-foreground" />,
}

export interface ReachabilityChecklistProps {
  /** Only these rows (e.g. 1–3 as People's empty state) */
  only?: RowKey[]
  /** Focus the relay field (row 1 without a URL) */
  onConnect?: () => void
  className?: string
}

/**
 * "Make this Nandi reachable": every opt-in a request from another Nandi
 * goes through, each with the one button that fixes it (network.md §4.2)
 */
export function ReachabilityChecklist({
  only,
  onConnect,
  className,
}: ReachabilityChecklistProps) {
  const { t } = useTranslation()
  const reachability = useReachability()
  const rows = only
    ? reachability.rows.filter(row => only.includes(row.key))
    : reachability.rows

  return (
    <section
      aria-labelledby="reachability-title"
      className={cn('flex flex-col gap-2', className)}
      data-testid="reachability"
    >
      <div className="flex items-baseline justify-between gap-2">
        <h2 id="reachability-title" className="text-sm font-semibold">
          {t('network.checklist.title')}
        </h2>
        <span className="text-xs text-muted-foreground">
          {t('network.checklist.done', {
            done: doneCount(rows),
            total: rows.length,
          })}
        </span>
      </div>
      <ol className="flex flex-col divide-y rounded-md border">
        {rows.map(row => (
          <li
            key={row.key}
            className="flex items-start gap-3 px-3 py-2.5"
            data-testid={`reachability-${row.key}`}
            data-status={row.status}
          >
            <span aria-hidden className="mt-0.5 shrink-0 [&_svg]:size-4">
              {ICONS[row.status]}
            </span>
            <RowBody
              row={row}
              reachability={reachability}
              onConnect={onConnect}
            />
          </li>
        ))}
      </ol>
    </section>
  )
}

type Reachability = ReturnType<typeof useReachability>

function RowBody({
  row,
  reachability,
  onConnect,
}: {
  row: ReachabilityRow
  reachability: Reachability
  onConnect?: () => void
}) {
  const { t } = useTranslation()
  const test = useTestRelayConnection()
  const makeReachable = useMakeGatewayReachable()
  const url = reachability.relay?.relayUrl ?? null

  const runTest = () => {
    if (!url) return
    test.mutate(url, {
      onSuccess: ms => toast.success(t('network.connection.testOk', { ms })),
      onError: error =>
        toast.error(t('network.connection.testFailed'), {
          description: error instanceof Error ? error.message : String(error),
        }),
    })
  }
  const testButton = (
    <Button
      size="sm"
      variant="outline"
      disabled={!url || test.isPending}
      onClick={runTest}
    >
      {test.isPending && <Spinner />}
      {t('network.connection.test')}
    </Button>
  )

  const title = rowTitle(t, row)
  let detail: React.ReactNode = null
  let fix: React.ReactNode = null

  switch (row.key) {
    case 'relay':
      if (row.status !== 'pass') {
        fix = url ? (
          testButton
        ) : (
          <Button size="sm" onClick={onConnect}>
            {t('network.connection.connect')}
          </Button>
        )
      }
      break
    case 'listed':
      if (row.status === 'blocked') {
        detail = t('network.checklist.connectFirst')
      } else if (row.status === 'fail') {
        fix = (
          <Button
            size="sm"
            disabled={makeReachable.isPending}
            onClick={() =>
              makeReachable.mutate(undefined, {
                onSuccess: status => {
                  reachability.markListed()
                  if (status.registered) {
                    toast.success(t('network.reachable.done'))
                  } else {
                    toast.warning(t('network.reachable.pending'))
                  }
                },
                onError: error =>
                  toast.error(
                    t('network.reachable.failed', {
                      error:
                        error instanceof Error ? error.message : String(error),
                    })
                  ),
              })
            }
          >
            {makeReachable.isPending && <Spinner />}
            {t('network.reachable.action')}
          </Button>
        )
      }
      break
    case 'confirmed':
      if (row.status === 'fail') {
        detail = t('network.checklist.notConfirmed')
        fix = testButton
      }
      break
    case 'direct':
      detail = t('network.checklist.directHint')
      fix = <ChooseAgents onNone={reachability.dismissDirect} />
      break
    case 'callers':
      if ((row.extra ?? 0) > 0) {
        detail = t('network.checklist.trustWaiting', { count: row.extra })
      }
      fix = (
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            navigate({ kind: 'network', tab: 'sharing', sharing: 'callers' })
          }
        >
          {t('network.checklist.review')}
        </Button>
      )
      break
    case 'approvals':
      if (row.status === 'fail') {
        fix = (
          <Button size="sm" onClick={() => navigate({ kind: 'home' })}>
            {t('network.checklist.openInbox')}
          </Button>
        )
      }
      break
    case 'visibility':
      if (row.status === 'info' && !row.hidden) {
        detail = t('network.checklist.notListedOthers', { count: row.extra })
      }
      break
  }

  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-center justify-between gap-2">
      <div className="min-w-0 space-y-0.5">
        <p className="text-sm">{title}</p>
        {detail && <p className="text-xs text-muted-foreground">{detail}</p>}
      </div>
      {fix}
    </div>
  )
}

function rowTitle(
  t: ReturnType<typeof useTranslation>['t'],
  row: ReachabilityRow
): React.ReactNode {
  const pass = row.status === 'pass'
  switch (row.key) {
    case 'relay':
      return pass ? (
        <>
          {t('network.checklist.relay.passPrefix')}{' '}
          <Term term="relay">{t('network.term.relayWord')}</Term>
        </>
      ) : (
        t('network.checklist.relay.fail')
      )
    case 'listed':
      return pass ? (
        <Term term="listed">{t('network.checklist.listed.pass')}</Term>
      ) : (
        t('network.checklist.listed.fail')
      )
    case 'confirmed':
      return t(
        pass
          ? 'network.checklist.confirmed.pass'
          : row.status === 'checking'
            ? 'network.checklist.confirmed.checking'
            : 'network.checklist.confirmed.fail'
      )
    case 'direct':
      return (row.count ?? 0) > 0 ? (
        <Term term="byName">
          {t('network.checklist.direct.pass', { count: row.count })}
        </Term>
      ) : (
        t('network.checklist.direct.fail')
      )
    case 'callers':
      return t('network.checklist.callers', { count: row.count })
    case 'approvals':
      return pass
        ? t('network.checklist.approvals.pass')
        : t('network.checklist.approvals.fail', { count: row.count })
    case 'visibility':
      if (row.hidden) return t('network.checklist.visibility.hidden')
      if (row.status === 'blocked' || row.status === 'checking') {
        return t('network.checklist.visibility.unknown')
      }
      return pass
        ? t('network.checklist.visibility.pass')
        : t('network.checklist.visibility.partial', {
            seen: row.count,
            total: (row.count ?? 0) + (row.extra ?? 0),
          })
  }
}

/** Row 4's picker: a switch per local agent for "takes requests by name" */
function ChooseAgents({ onNone }: { onNone: () => void }) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="sm" variant="outline">
          {t('network.checklist.chooseAgents')}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 space-y-3">
        <p className="text-sm font-medium">
          {t('network.checklist.chooseAgentsTitle')}
        </p>
        {agents.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('network.checklist.noAgents')}
          </p>
        ) : (
          <ul className="space-y-2">
            {agents.map(agent => (
              <AgentSwitch
                key={agent.id}
                agentId={agent.id}
                name={agent.name}
              />
            ))}
          </ul>
        )}
        <Button size="sm" variant="ghost" className="w-full" onClick={onNone}>
          {t('network.checklist.noneByName')}
        </Button>
      </PopoverContent>
    </Popover>
  )
}

function AgentSwitch({ agentId, name }: { agentId: string; name: string }) {
  const { t } = useTranslation()
  const settings = useAgentDelegation(agentId)
  const save = useSetAgentDelegation(agentId)
  const id = `accept-${agentId}`
  return (
    <li className="flex items-center justify-between gap-2">
      <label htmlFor={id} className="truncate text-sm">
        {name}
      </label>
      <Switch
        id={id}
        checked={settings.data?.acceptInbound ?? false}
        disabled={!settings.data || save.isPending}
        aria-label={t('network.checklist.takesRequestsFor', { name })}
        onCheckedChange={acceptInbound =>
          settings.data &&
          save.mutate(
            { ...settings.data, acceptInbound },
            {
              onError: error =>
                toast.error(
                  error instanceof Error ? error.message : String(error)
                ),
            }
          )
        }
      />
    </li>
  )
}
