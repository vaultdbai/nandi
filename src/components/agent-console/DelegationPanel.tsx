/**
 * Context-panel section for one agent: the outbound/inbound delegation
 * toggles (both off by default), per-capability exceptions, the agents it
 * can call (by capability), and the agents that can call it.
 *
 * Settings live in the agent's skill.md; saving goes through
 * `setAgentDelegation`, which writes the file.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ArrowDownLeft, ArrowUpRight, Network, Plus, X } from 'lucide-react'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { useNavigationStore } from '@/store/navigation-store'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import {
  agentIdOf,
  callersOf,
  errorMessage,
  ownCard,
  providersByCapability,
  useAgentDelegation,
  useAvailableProviders,
  useDelegation,
  useSetAgentDelegation,
  useTrustRules,
} from '@/hooks/useDelegation'
import type {
  AgentDelegationSettings,
  DelegationOverride,
  TrustPolicy,
} from '@/lib/bindings'

const POLICIES: TrustPolicy[] = ['allow', 'deny', 'ask']

export function DelegationPanel({
  agentId,
  agentName,
}: {
  agentId: string
  agentName: string
}) {
  const { t } = useTranslation()
  const settings = useAgentDelegation(agentId)
  const save = useSetAgentDelegation(agentId)
  const trust = useTrustRules()
  // Turning on "accepts requests" lets other Nandis send work: ask once
  const [confirmInbound, setConfirmInbound] = useState(false)

  if (settings.isPending) {
    return (
      <div className="flex flex-col gap-2 p-4" aria-busy="true">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-10 w-full" />
      </div>
    )
  }
  if (settings.isError) {
    return (
      <p role="alert" className="p-4 text-xs text-destructive">
        {errorMessage(settings.error)}
      </p>
    )
  }

  const current = settings.data
  const update = (change: Partial<AgentDelegationSettings>) => {
    save.mutate(
      { ...current, ...change },
      {
        onError: error =>
          toast.error(
            t('delegation.panel.saveFailed', { error: errorMessage(error) })
          ),
      }
    )
  }

  return (
    <section
      aria-labelledby={`delegation-${agentId}`}
      className="flex flex-col gap-5 border-t p-4"
    >
      <div className="flex flex-col gap-1">
        <h2
          id={`delegation-${agentId}`}
          className="flex items-center gap-2 text-sm font-medium"
        >
          <Network className="size-4 text-muted-foreground" />
          {t('delegation.panel.title')}
        </h2>
        <p className="text-xs text-muted-foreground">
          {t('delegation.panel.description')}
        </p>
      </div>

      <Toggle
        id={`delegation-outbound-${agentId}`}
        label={t('delegation.panel.outbound')}
        hint={t('delegation.panel.outboundHint')}
        checked={current.allowOutbound}
        onChange={allowOutbound => update({ allowOutbound })}
      />
      <Toggle
        id={`delegation-inbound-${agentId}`}
        label={t('delegation.panel.inbound')}
        hint={t('delegation.panel.inboundHint')}
        checked={current.acceptInbound}
        onChange={acceptInbound =>
          acceptInbound ? setConfirmInbound(true) : update({ acceptInbound })
        }
      />
      {current.acceptInbound && (
        <p className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
          {t('agentPage.sharing.whoCanSend', {
            count: (trust.data ?? []).filter(rule => rule.policy === 'allow')
              .length,
          })}
          <Button
            variant="link"
            size="sm"
            className="h-auto px-0 text-xs"
            onClick={() =>
              useNavigationStore
                .getState()
                .navigate({ kind: 'network', tab: 'sharing' })
            }
          >
            {t('agentPage.sharing.manage')}
          </Button>
        </p>
      )}
      <AlertDialog open={confirmInbound} onOpenChange={setConfirmInbound}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('agentPage.sharing.confirmTitle', { name: agentName })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('agentPage.sharing.confirmDescription', { name: agentName })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('agentPage.sharing.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => update({ acceptInbound: true })}>
              {t('agentPage.sharing.turnOn')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      {current.acceptInbound && (
        <div className="flex items-center justify-between gap-3">
          <label
            htmlFor={`delegation-max-${agentId}`}
            className="text-xs text-muted-foreground"
          >
            {t('delegation.panel.maxConcurrent')}
          </label>
          <Input
            id={`delegation-max-${agentId}`}
            type="number"
            min={1}
            max={64}
            className="h-8 w-20"
            value={current.maxConcurrent ?? 1}
            onChange={event => {
              const value = Number.parseInt(event.target.value, 10)
              if (Number.isFinite(value) && value >= 1) {
                update({ maxConcurrent: value })
              }
            }}
          />
        </div>
      )}

      <Overrides
        agentId={agentId}
        overrides={current.overrides}
        onChange={overrides => update({ overrides })}
      />
      <CanCall agentId={agentId} outbound={current.allowOutbound} />
      <Callers agentId={agentId} />
    </section>
  )
}

function Toggle({
  id,
  label,
  hint,
  checked,
  onChange,
}: {
  id: string
  label: string
  hint: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <label htmlFor={id} className="text-sm">
          {label}
        </label>
        <p className="text-xs text-muted-foreground">{hint}</p>
      </div>
      <Switch id={id} checked={checked} onCheckedChange={onChange} />
    </div>
  )
}

function Overrides({
  agentId,
  overrides,
  onChange,
}: {
  agentId: string
  overrides: DelegationOverride[]
  onChange: (overrides: DelegationOverride[]) => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState('')
  const providers = useAvailableProviders('')
  const served =
    ownCard(providers.data ?? [], agentId)?.capabilities.map(c => c.name) ?? []
  const listed = new Set(overrides.map(o => o.capability))
  const rows: DelegationOverride[] = [
    ...overrides,
    ...served
      .filter(capability => !listed.has(capability))
      .map(capability => ({ capability, policy: 'ask' as TrustPolicy })),
  ]

  const setPolicy = (capability: string, policy: TrustPolicy) => {
    const rest = overrides.filter(o => o.capability !== capability)
    // `ask` is the absence of an exception
    onChange(policy === 'ask' ? rest : [...rest, { capability, policy }])
  }

  const addDraft = () => {
    const capability = draft.trim()
    if (!capability || /\s/.test(capability) || listed.has(capability)) return
    onChange([...overrides, { capability, policy: 'allow' }])
    setDraft('')
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-col gap-0.5">
        <h3 className="text-xs font-medium">
          {t('delegation.panel.overrides')}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t('delegation.panel.overridesHint')}
        </p>
      </div>
      {rows.map(row => (
        <div
          key={row.capability}
          className="flex items-center justify-between gap-2"
        >
          <span className="truncate font-mono text-xs" dir="ltr">
            {row.capability}
          </span>
          <div className="flex items-center gap-1">
            <NativeSelect
              aria-label={t('delegation.panel.overridePolicy', {
                capability: row.capability,
              })}
              value={row.policy}
              onChange={event =>
                setPolicy(row.capability, event.target.value as TrustPolicy)
              }
            >
              {POLICIES.map(policy => (
                <NativeSelectOption key={policy} value={policy}>
                  {t(`delegation.policy.${policy}`)}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            {listed.has(row.capability) && (
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t('delegation.panel.removeOverride', {
                  capability: row.capability,
                })}
                onClick={() => setPolicy(row.capability, 'ask')}
              >
                <X />
              </Button>
            )}
          </div>
        </div>
      ))}
      <form
        className="flex items-center gap-2"
        onSubmit={event => {
          event.preventDefault()
          addDraft()
        }}
      >
        <Input
          dir="ltr"
          className="h-8 font-mono text-xs"
          value={draft}
          placeholder={t('delegation.panel.overridePlaceholder')}
          aria-label={t('delegation.panel.overridePlaceholder')}
          onChange={event => setDraft(event.target.value)}
        />
        <Button type="submit" size="sm" variant="outline">
          <Plus />
          {t('delegation.panel.addOverride')}
        </Button>
      </form>
    </div>
  )
}

function CanCall({
  agentId,
  outbound,
}: {
  agentId: string
  outbound: boolean
}) {
  const { t } = useTranslation()
  const providers = useAvailableProviders('')
  const groups = providersByCapability(providers.data ?? [], agentId)

  return (
    <div className="flex flex-col gap-2">
      <h3 className="flex items-center gap-1.5 text-xs font-medium">
        <ArrowUpRight className="size-3.5 text-muted-foreground rtl:-scale-x-100" />
        {t('delegation.panel.canCall')}
      </h3>
      {!outbound && groups.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {t('delegation.panel.outboundOff')}
        </p>
      )}
      {providers.isError && (
        <p className="text-xs text-muted-foreground">
          {t('delegation.panel.canCallUnavailable', {
            error: errorMessage(providers.error),
          })}
        </p>
      )}
      {providers.isSuccess && groups.length === 0 && (
        <p className="text-xs text-muted-foreground">
          {t('delegation.panel.canCallEmpty')}
        </p>
      )}
      {groups.map(([capability, cards]) => (
        <div key={capability} className="flex flex-col gap-1">
          <span className="font-mono text-xs text-muted-foreground" dir="ltr">
            {capability}
          </span>
          <ul className="flex flex-col gap-1 ps-3">
            {cards.map(card => (
              <li
                key={card.id}
                className="flex items-center justify-between gap-2 text-xs"
              >
                <span className="min-w-0 truncate" title={card.id}>
                  {card.name}
                  {card.owner.githubHandle && (
                    <span className="text-muted-foreground">
                      {' '}
                      @{card.owner.githubHandle}
                    </span>
                  )}
                </span>
                {card.acceptsDelegation ? (
                  <span className="shrink-0 text-muted-foreground tabular-nums">
                    {t('delegation.panel.load', {
                      load: card.currentLoad,
                      max: card.maxConcurrent,
                    })}
                  </span>
                ) : (
                  <Badge variant="outline" className="shrink-0">
                    {t('delegation.panel.notAccepting')}
                  </Badge>
                )}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  )
}

function Callers({ agentId }: { agentId: string }) {
  const { t } = useTranslation()
  const rules = useTrustRules()
  const { history } = useDelegation()
  const callers = callersOf(agentId, rules.data ?? [], history)

  return (
    <div className="flex flex-col gap-2">
      <h3 className="flex items-center gap-1.5 text-xs font-medium">
        <ArrowDownLeft className="size-3.5 text-muted-foreground rtl:-scale-x-100" />
        {t('delegation.panel.callers')}
      </h3>
      {callers.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          {t('delegation.panel.callersEmpty')}
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {callers.map(caller => (
            <li key={caller} className="text-xs" title={caller}>
              {caller === '*' ? (
                t('delegation.panel.anyone')
              ) : (
                <span className="font-mono" dir="ltr">
                  {agentIdOf(caller)}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
