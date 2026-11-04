/**
 * Network › Connection: the relay ("Join a company relay" with its address
 * and join code, status, Test, Copy invite, Disconnect), the
 * "Make this Nandi reachable" checklist, and Details, the only place the
 * device ID shows (network.md §4.2).
 */
import { useRef, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { ChevronRight, Copy, Unplug } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { ConfirmAction } from '@/components/network/ConfirmAction'
import {
  JoinRelayFields,
  JoinResult,
  type JoinDraft,
} from '@/components/network/JoinRelayFields'
import { joinMessage, type JoinMessage } from '@/components/network/join-model'
import { HealthDot, Term } from '@/components/network/NetworkHelp'
import {
  isRelayUrl,
  isUnencryptedRemoteRelay,
  withoutToken,
} from '@/components/onboarding/identity-model'
import { useNetworkHealth } from '@/hooks/useNetworkHealth'
import { ReachabilityChecklist } from '@/components/network/ReachabilityChecklist'
import { errorMessage } from '@/hooks/useDelegation'
import { useGatewayManifest, useSetGatewayManifest } from '@/hooks/useGateway'
import {
  DEFAULT_RELAY_URL,
  useCopyInvite,
  useJoinRelay,
  useRelay,
  useRelayInstances,
  useRelayTrust,
  RELAY_TRUST_KEY,
  useSetRelayUrl,
  useTestRelayConnection,
} from '@/hooks/useRelay'
import type { RelayStatus } from '@/lib/bindings'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'

const DETAILS_OPEN = 'nandi.network.detailsOpen'

function readDetailsOpen(): boolean {
  try {
    return localStorage.getItem(DETAILS_OPEN) === '1'
  } catch {
    return false
  }
}

export function ConnectionView() {
  const { status, isLoading } = useRelay()
  const fieldRef = useRef<HTMLInputElement>(null)

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 p-6">
        {isLoading ? (
          <Skeleton className="h-40 w-full" />
        ) : (
          <RelayCard status={status} fieldRef={fieldRef} />
        )}
        <RelayTrustCard />
        <ReachabilityChecklist onConnect={() => fieldRef.current?.focus()} />
        <ConnectionDetails status={status} />
      </div>
    </ScrollArea>
  )
}

/** Shown for a plain `ws://` relay on another machine */
function UnencryptedWarning({ id }: { id?: string }) {
  const { t } = useTranslation()
  return (
    <p id={id} className="text-sm text-warning" data-testid="relay-unencrypted">
      {t('network.connection.unencrypted')}
    </p>
  )
}

function RelayCard({
  status,
  fieldRef,
}: {
  status: RelayStatus | undefined
  fieldRef: React.RefObject<HTMLInputElement | null>
}) {
  const { t, i18n } = useTranslation()
  const { health, label } = useNetworkHealth()
  const setUrl = useSetRelayUrl()
  const join = useJoinRelay()
  const test = useTestRelayConnection()
  const copyInvite = useCopyInvite()
  const [result, setResult] = useState<JoinMessage | null>(null)
  const saved = status?.relayUrl ?? null
  const fromEnv = status?.relayUrlSource === 'env'
  // A saved address is text until Change…; none yet: the field, prefilled
  const [draft, setDraft] = useState<JoinDraft | null>(null)
  const editing = !saved || draft !== null
  const value = draft ?? { url: DEFAULT_RELAY_URL, joinCode: '' }
  const connected = Boolean(status?.connected)
  const instances = useRelayInstances(connected)
  const nandis = instances.data?.length ?? status?.instancesOnline ?? 0

  const disconnect = () =>
    setUrl.mutate('', {
      onSuccess: () => {
        setDraft(null)
        setResult(null)
        toast.success(t('relay.url.cleared'))
      },
      onError: error =>
        toast.error(t('relay.url.error'), { description: errorMessage(error) }),
    })

  const submit = () => {
    const url = value.url.trim()
    if (!isRelayUrl(url)) return
    setResult(null)
    join.mutate(
      { url, joinCode: value.joinCode },
      {
        onSuccess: outcome => {
          const message = joinMessage(outcome, url)
          if (!message.ok) return setResult(message)
          // The code is never kept on screen once used
          setDraft(null)
          toast.success(t(message.key, message.params))
        },
        onError: error =>
          toast.error(t('relay.url.error'), {
            description: errorMessage(error),
          }),
      }
    )
  }

  const edit = (next: JoinDraft) => {
    setDraft(next)
    setResult(null)
    test.reset()
  }

  const runTest = () =>
    test.mutate((editing ? value.url : withoutToken(saved ?? '')).trim())

  return (
    <section
      aria-labelledby="relay-title"
      className="flex flex-col gap-3 rounded-md border p-4"
      data-testid="relay-card"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="relay-title" className="text-sm font-semibold">
          <Term term="relay">{t('network.connection.relayTitle')}</Term>
        </h2>
        <span
          className="inline-flex items-center gap-2 text-sm"
          data-testid="relay-state"
          data-state={status?.state ?? 'disconnected'}
        >
          <HealthDot health={health} />
          {t(label)}
        </span>
      </div>

      {status?.state === 'error' && (
        <p className="text-sm text-destructive" data-testid="relay-error">
          {status.versionMismatch
            ? t('network.connection.versionMismatch')
            : t('network.connection.errorSummary')}
        </p>
      )}

      {editing ? (
        <form
          className="grid gap-3"
          aria-label={t('relay.join.title')}
          onSubmit={event => {
            event.preventDefault()
            submit()
          }}
        >
          <div className="space-y-1">
            <h3 className="text-sm font-medium">{t('relay.join.title')}</h3>
            {!saved && (
              <p className="text-sm text-muted-foreground">
                {t('network.connection.firstRun')}
              </p>
            )}
          </div>
          <JoinRelayFields
            idPrefix="relay"
            urlLabel={t('relay.url.label')}
            urlRef={fieldRef}
            value={value}
            onChange={edit}
            invalid={!isRelayUrl(value.url)}
            describedBy={
              isUnencryptedRemoteRelay(value.url)
                ? 'relay-unencrypted'
                : undefined
            }
          />
          {isUnencryptedRemoteRelay(value.url) && (
            <UnencryptedWarning id="relay-unencrypted" />
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              type="submit"
              size="sm"
              disabled={join.isPending || !isRelayUrl(value.url)}
            >
              {join.isPending && <Spinner />}
              {t('relay.join.join')}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={test.isPending || !isRelayUrl(value.url)}
              onClick={runTest}
            >
              {test.isPending && <Spinner />}
              {t('network.connection.test')}
            </Button>
            {saved && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={() => {
                  setDraft(null)
                  setResult(null)
                  test.reset()
                }}
              >
                {t('network.cancel')}
              </Button>
            )}
          </div>
          <JoinResult pending={false} message={result} />
        </form>
      ) : (
        <div className="grid gap-2">
          <p
            dir="ltr"
            className="font-mono text-sm break-all"
            data-testid="relay-url"
          >
            {/* Never this instance's token: it's on screen and in screenshots */}
            {saved && withoutToken(saved)}
          </p>
          {saved && isUnencryptedRemoteRelay(saved) && <UnencryptedWarning />}
          <p className="text-xs text-muted-foreground">
            {t('network.connection.summary', {
              heartbeat: status?.lastHeartbeatAt
                ? formatRelativeTime(status.lastHeartbeatAt, i18n.language)
                : t('relay.status.never'),
              count: nandis,
            })}
          </p>
          {fromEnv && (
            <p className="text-xs text-muted-foreground">
              {t('network.connection.fromEnv')}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={fromEnv}
              onClick={() =>
                setDraft({ url: withoutToken(saved ?? ''), joinCode: '' })
              }
            >
              {t('network.connection.change')}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void copyInvite()}
            >
              <Copy />
              {t('network.invite.copy')}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={test.isPending}
              onClick={runTest}
            >
              {test.isPending && <Spinner />}
              {t('network.connection.test')}
            </Button>
            <ConfirmAction
              trigger={
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={fromEnv || setUrl.isPending}
                >
                  <Unplug />
                  {t('relay.url.disconnect')}
                </Button>
              }
              title={t('network.connection.disconnectTitle')}
              description={t('network.connection.disconnectBody')}
              confirmLabel={t('relay.url.disconnect')}
              onConfirm={disconnect}
            />
          </div>
        </div>
      )}

      {(test.isSuccess || test.isError) && (
        <p
          role="status"
          data-testid="relay-test-result"
          className={cn(
            'text-xs break-words',
            test.isSuccess ? 'text-success' : 'text-destructive'
          )}
        >
          {test.isSuccess
            ? t('network.connection.testOk', { ms: test.data })
            : t('network.connection.testFailed')}
        </p>
      )}
    </section>
  )
}

/**
 * "Trust everyone on this relay": on a relay that needs a token, its
 * members are the company's, so they're trusted unless this is off
 * (`team::trust::is_trusted`). On an open relay it doesn't apply.
 */
function RelayTrustCard() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const trust = useRelayTrust()
  const save = useMutation({
    mutationFn: async (enabled: boolean) =>
      unwrapResult(await client.setRelayTrust(enabled)),
    onSuccess: next => queryClient.setQueryData(RELAY_TRUST_KEY, next),
    onError: error =>
      toast.error(t('network.connection.relayTrust.error'), {
        description: errorMessage(error),
      }),
  })
  if (!trust.data) return null
  const { enabled, secured } = trust.data
  return (
    <section
      aria-labelledby="relay-trust-title"
      className="flex items-start justify-between gap-4 rounded-md border p-4"
      data-testid="relay-trust"
    >
      <div className="space-y-1">
        <Label id="relay-trust-title" htmlFor="relay-trust">
          {t('network.connection.relayTrust.title')}
        </Label>
        <p className="text-sm text-muted-foreground">
          {secured
            ? t('network.connection.relayTrust.secured')
            : t('network.connection.relayTrust.open')}
        </p>
      </div>
      <Switch
        id="relay-trust"
        checked={enabled}
        disabled={!secured || save.isPending}
        onCheckedChange={checked => save.mutate(checked)}
      />
    </section>
  )
}

function ConnectionDetails({ status }: { status: RelayStatus | undefined }) {
  const { t, i18n } = useTranslation()
  const [open, setOpen] = useState(readDetailsOpen)
  const manifest = useGatewayManifest()
  const saveManifest = useSetGatewayManifest()
  const deviceId = status?.instanceId ?? ''
  const source = status?.relayUrlSource
  const when = (time: number | null | undefined) =>
    time ? formatRelativeTime(time, i18n.language) : t('relay.status.never')

  const toggle = (next: boolean) => {
    setOpen(next)
    try {
      localStorage.setItem(DETAILS_OPEN, next ? '1' : '0')
    } catch {
      // Not remembered: Details just starts closed next time
    }
  }

  return (
    <Collapsible open={open} onOpenChange={toggle}>
      <CollapsibleTrigger asChild>
        <Button variant="ghost" size="sm" className="gap-1 px-1">
          <ChevronRight
            className={cn(
              'transition-transform rtl:rotate-180',
              open && 'rotate-90'
            )}
          />
          {t('network.details.title')}
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <dl
          className="mt-2 grid gap-3 rounded-md border p-4 text-sm"
          data-testid="connection-details"
        >
          <div className="grid gap-1">
            <dt className="text-xs text-muted-foreground">
              <Term term="device">{t('network.details.deviceId')}</Term>
            </dt>
            <dd className="flex flex-wrap items-center gap-2">
              <code dir="ltr" className="font-mono break-all select-all">
                {deviceId || '—'}
              </code>
              {deviceId && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    void navigator.clipboard?.writeText(deviceId)
                    toast.success(t('network.copied'))
                  }}
                >
                  <Copy />
                  {t('network.copy')}
                </Button>
              )}
            </dd>
          </div>
          <div className="grid gap-1">
            <dt className="text-xs text-muted-foreground">
              {t('network.details.source')}
            </dt>
            <dd>
              {source
                ? t(`network.details.sourceKind.${source}`)
                : t('relay.status.noUrl')}
            </dd>
          </div>
          <div className="grid gap-1">
            <dt className="text-xs text-muted-foreground">
              {t('relay.status.lastConnected')}
            </dt>
            <dd data-testid="relay-last-connected">
              {when(status?.lastConnectedAt)}
            </dd>
          </div>
          <div className="grid gap-1">
            <dt className="text-xs text-muted-foreground">
              {t('network.details.agentsOnline')}
            </dt>
            <dd>{status?.agentsOnline ?? 0}</dd>
          </div>
          {status?.reason && (
            <div className="grid gap-1">
              <dt className="text-xs text-muted-foreground">
                {t('network.details.reason')}
              </dt>
              <dd className="break-words" data-testid="relay-reason">
                {status.reason}
              </dd>
            </div>
          )}
          {manifest.data && (
            <div className="flex items-center justify-between gap-3 border-t pt-3">
              <Label htmlFor="forward-trusted" className="font-normal">
                {t('network.details.forwardTrusted')}
              </Label>
              <Switch
                id="forward-trusted"
                checked={manifest.data.federation.enabled}
                disabled={saveManifest.isPending}
                onCheckedChange={enabled =>
                  manifest.data &&
                  saveManifest.mutate(
                    {
                      ...manifest.data,
                      federation: { ...manifest.data.federation, enabled },
                    },
                    {
                      onError: error => toast.error(errorMessage(error)),
                    }
                  )
                }
              />
            </div>
          )}
        </dl>
      </CollapsibleContent>
    </Collapsible>
  )
}
