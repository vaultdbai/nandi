import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ArrowRight } from 'lucide-react'
import {
  accessMode,
  blockedSite,
} from '@/components/agents/network-access-model'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { useAgentDefinition, useAllowSite } from '@/hooks/useAgentEditor'
import { useAgents } from '@/hooks/useAgents'
import { useEgressStatus } from '@/hooks/useEgressStatus'
import { formatRelativeTime } from '@/lib/format-time'
import { openSettings } from '@/lib/settings-nav'
import type {
  AgentInfo,
  EgressLogEntry,
  EgressRoute,
} from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import {
  AdvancedBlock,
  SettingsGroup,
  SettingsSectionFrame,
} from '../SettingsFrame'

/**
 * Settings › Internet access: each agent's allowed sites (edited on the
 * agent, not here), the latest requests, and the proxy under Advanced
 */
export function InternetAccessSection({ agentId }: { agentId?: string }) {
  const { t } = useTranslation()
  const agents = useAgents()
  const { data: status, error } = useEgressStatus()
  const all = agents.data ?? []
  const shown = agentId ? all.filter(a => a.id === agentId) : all

  return (
    <SettingsSectionFrame
      title={t('settings.section.internet')}
      description={t('settings.internet.description')}
    >
      <SettingsGroup title={t('settings.internet.byAgent')}>
        <div className="flex justify-end">
          <NativeSelect
            aria-label={t('settings.internet.agentFilter')}
            value={agentId ?? ''}
            onChange={event =>
              openSettings(
                'internet',
                event.target.value ? { agentId: event.target.value } : {}
              )
            }
          >
            <NativeSelectOption value="">
              {t('settings.internet.allAgents')}
            </NativeSelectOption>
            {all.map(agent => (
              <NativeSelectOption key={agent.id} value={agent.id}>
                {agent.name}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
        {agents.isPending ? (
          <Skeleton className="h-24" />
        ) : all.length === 0 ? (
          <div className="flex items-center gap-3 rounded-lg border p-4 text-sm text-muted-foreground">
            <span className="flex-1">{t('settings.internet.noAgents')}</span>
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                useNavigationStore.getState().navigate({ kind: 'agents' })
              }
            >
              {t('settings.internet.createAgent')}
            </Button>
          </div>
        ) : (
          <ul className="divide-y rounded-lg border">
            {shown.map(agent => (
              <AllowedSitesRow key={agent.id} agent={agent} />
            ))}
          </ul>
        )}
      </SettingsGroup>

      <SettingsGroup title={t('settings.internet.recent')}>
        <div className="flex justify-end">
          <Button
            size="sm"
            variant="link"
            className="h-auto p-0"
            onClick={() => openSettings('logs', { logTab: 'egress' })}
          >
            {t('settings.internet.seeAll')}
            <ArrowRight className="rtl:rotate-180" />
          </Button>
        </div>
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {t('settings.internet.proxyDown')}
          </p>
        ) : !status ? (
          <Skeleton className="h-24" />
        ) : (
          <RecentList entries={status.recentEntries} agents={all} />
        )}
      </SettingsGroup>

      {status && (
        <AdvancedBlock>
          <Routes routes={status.routes} proxyUrl={status.proxyUrl} />
        </AdvancedBlock>
      )}
    </SettingsSectionFrame>
  )
}

/** One agent's allowed sites, from its definition, with Edit › */
function AllowedSitesRow({ agent }: { agent: AgentInfo }) {
  const { t } = useTranslation()
  const definition = useAgentDefinition(agent.id)
  const sites = definition.data?.draft.network ?? []

  return (
    <li
      className="flex flex-wrap items-center gap-3 px-4 py-3"
      data-settings-focus={agent.id}
    >
      <span className="w-40 shrink-0 truncate text-sm font-medium">
        {agent.name}
      </span>
      <span className="min-w-0 flex-1 truncate text-sm text-muted-foreground">
        {definition.isPending ? (
          <Skeleton className="h-4 w-32" />
        ) : sites.length === 0 ? (
          t('settings.internet.noAccess')
        ) : accessMode(sites) === 'any' ? (
          t('settings.internet.anySite')
        ) : (
          <span dir="ltr" className="font-mono text-xs">
            {sites.join(', ')}
          </span>
        )}
      </span>
      <Button
        size="sm"
        variant="ghost"
        onClick={() =>
          useNavigationStore
            .getState()
            .navigate({ kind: 'agent', agentId: agent.id, tab: 'settings' })
        }
      >
        {t('settings.internet.edit')}
        <ArrowRight className="rtl:rotate-180" />
      </Button>
    </li>
  )
}

function RecentList({
  entries,
  agents,
}: {
  entries: EgressLogEntry[]
  agents: AgentInfo[]
}) {
  const { t, i18n } = useTranslation()
  const byId = new Map(agents.map(a => [a.id, a]))
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {t('settings.internet.noRequests')}
      </p>
    )
  }
  return (
    <ul className="flex flex-col divide-y rounded-lg border">
      {entries.map((entry, index) => (
        <li
          key={`${entry.ts}-${index}`}
          className="flex items-center gap-3 px-4 py-2 text-sm"
        >
          <span className="w-20 shrink-0 text-xs text-muted-foreground tabular-nums">
            {formatRelativeTime(entry.ts, i18n.language)}
          </span>
          <span className="w-32 shrink-0 truncate">
            {byId.get(entry.agentId)?.name ?? entry.agentId}
          </span>
          <span
            dir="ltr"
            className="min-w-0 flex-1 truncate font-mono text-xs"
            title={entry.target}
          >
            {entry.target}
          </span>
          <Badge
            variant="outline"
            className={cn(
              'shrink-0',
              entry.allowed ? 'text-success' : 'text-destructive'
            )}
          >
            {t(entry.allowed ? 'egress.log.allowed' : 'egress.log.denied')}
          </Badge>
          <AllowButton entry={entry} agent={byId.get(entry.agentId)} />
        </li>
      ))}
    </ul>
  )
}

/**
 * Allow on a blocked request: adds its site to the agent's allowed sites.
 * Only for agents made here; installed ones are changed in a copy.
 */
function AllowButton({
  entry,
  agent,
}: {
  entry: EgressLogEntry
  agent: AgentInfo | undefined
}) {
  const site = blockedSite(entry)
  if (!site || agent?.origin.kind !== 'local') return null
  return <AllowSite agent={agent} site={site} />
}

function AllowSite({ agent, site }: { agent: AgentInfo; site: string }) {
  const { t } = useTranslation()
  const allow = useAllowSite(agent.id)
  return (
    <Button
      size="sm"
      variant="outline"
      className="shrink-0"
      disabled={allow.isPending}
      onClick={() =>
        allow.mutate(site, {
          onSuccess: () =>
            toast.success(
              t('settings.internet.allowDone', { site, agent: agent.name })
            ),
          onError: error =>
            toast.error(t('settings.internet.allowFailed', { site }), {
              description: error.message,
            }),
        })
      }
    >
      {t('settings.internet.allow')}
    </Button>
  )
}

function Routes({
  routes,
  proxyUrl,
}: {
  routes: EgressRoute[]
  proxyUrl: string
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-2">
      <p className="text-sm text-muted-foreground">
        {t('egress.routes.description', { url: proxyUrl })}
      </p>
      <ul className="flex flex-col gap-3">
        {routes.map(route => (
          <li key={route.kind} className="flex flex-col gap-0.5">
            <code dir="ltr" className="text-sm">
              <span className="font-semibold">{route.method}</span> {route.path}
            </code>
            <span className="text-xs text-muted-foreground">
              {t(`egress.routes.${route.kind}`)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}
