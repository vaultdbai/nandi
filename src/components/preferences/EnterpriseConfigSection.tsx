import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useRepoLlmConfig, useSyncRepoLlmConfig } from '@/hooks/useRepoConfig'
import type { EffectiveValue, LlmProviderConfig } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { SettingsSection } from './shared/SettingsComponents'

/**
 * Enterprise config: the team's `.nandi/llm.yaml` (providers, routing) and
 * what's in effect after environment > repo > local, with Sync
 */
export function EnterpriseConfigSection() {
  const { t, i18n } = useTranslation()
  const { data: status, error } = useRepoLlmConfig()
  const sync = useSyncRepoLlmConfig()

  const onSync = () =>
    sync.mutate(undefined, {
      onSuccess: next =>
        next.error
          ? toast.error(t('preferences.enterprise.invalid'))
          : toast.success(t('preferences.enterprise.synced')),
      onError: e =>
        toast.error(
          t('preferences.enterprise.syncFailed', {
            error: e instanceof Error ? e.message : String(e),
          })
        ),
    })

  return (
    <SettingsSection title={t('preferences.enterprise.title')}>
      <div className="flex flex-wrap items-start gap-3">
        <p className="min-w-0 flex-1 text-sm text-muted-foreground">
          {t('preferences.enterprise.description')}
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={onSync}
          disabled={sync.isPending}
        >
          <RefreshCw className={sync.isPending ? 'animate-spin' : undefined} />
          {t('preferences.enterprise.sync')}
        </Button>
      </div>
      {error && (
        <Alert variant="destructive">
          <AlertDescription>
            {error instanceof Error ? error.message : String(error)}
          </AlertDescription>
        </Alert>
      )}
      {status && !status.found && (
        <p
          className="text-sm text-muted-foreground"
          data-testid="enterprise-none"
        >
          {t('preferences.enterprise.none', { path: '.nandi/llm.yaml' })}
        </p>
      )}
      {status?.error && (
        <Alert variant="destructive">
          <AlertDescription>{status.error}</AlertDescription>
        </Alert>
      )}
      {status?.config && (
        <ul className="flex flex-col gap-2" data-testid="enterprise-providers">
          {status.config.providers.map(provider => (
            <ProviderRow
              key={provider.id}
              provider={provider}
              role={
                provider.id === status.config?.defaultProvider
                  ? t('preferences.enterprise.default')
                  : status.config?.fallbacks.includes(provider.id)
                    ? t('preferences.enterprise.fallback')
                    : null
              }
            />
          ))}
        </ul>
      )}
      {status && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          <Effective
            label={t('preferences.enterprise.provider')}
            value={status.effectiveProvider}
          />
          <Effective
            label={t('preferences.enterprise.region')}
            value={status.effectiveRegion}
          />
          <Effective
            label={t('preferences.enterprise.profile')}
            value={status.effectiveProfile}
          />
          <Effective
            label={t('preferences.enterprise.model')}
            value={status.effectiveModel}
          />
        </dl>
      )}
      {status?.syncedAt != null && (
        <p className="text-xs text-muted-foreground">
          {t('preferences.enterprise.syncedAt', {
            when: formatRelativeTime(status.syncedAt, i18n.language),
          })}
        </p>
      )}
    </SettingsSection>
  )
}

function ProviderRow({
  provider,
  role,
}: {
  provider: LlmProviderConfig
  role: string | null
}) {
  const details = [
    provider.region,
    provider.profile,
    provider.endpoint,
    provider.model,
  ].filter(Boolean)
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-md border px-3 py-2 text-sm">
      <span className="font-medium">{provider.id}</span>
      <Badge variant="outline">{provider.kind}</Badge>
      {role && <Badge variant="secondary">{role}</Badge>}
      <span className="min-w-0 truncate text-muted-foreground" dir="ltr">
        {details.join(' · ')}
      </span>
    </li>
  )
}

function Effective({
  label,
  value,
}: {
  label: string
  value: EffectiveValue | null
}) {
  const { t } = useTranslation()
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="flex flex-wrap items-center gap-2">
        {value ? (
          <>
            <span dir="ltr">{value.value}</span>
            <Badge variant="outline" className="text-xs">
              {t(`preferences.enterprise.source.${value.source}`)}
            </Badge>
          </>
        ) : (
          <span className="text-muted-foreground">—</span>
        )}
      </dd>
    </>
  )
}
