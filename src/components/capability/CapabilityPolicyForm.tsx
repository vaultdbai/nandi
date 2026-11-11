import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { useSaveCapability } from '@/hooks/useCapabilities'
import { rawError } from '@/lib/friendly-error'
import type {
  CapabilityDefinition,
  CapabilityPolicy,
  CapabilityRouteStrategy,
  FallbackPolicy,
  ReassignPolicy,
  RoutingPolicy,
  Visibility,
} from '@/lib/bindings'

/** The core's defaults (`CapabilityPolicy::default`, `RoutingPolicy::default`) */
const DEFAULT_POLICY: CapabilityPolicy = {
  visibility: 'private',
  maxConcurrentPerProvider: 1,
  totalMaxConcurrent: 0,
  slaSeconds: 0,
  approvalRequired: false,
  cost: null,
  autoReassign: 'never',
}

const DEFAULT_ROUTING: RoutingPolicy = {
  strategy: 'least_loaded',
  fallback: 'none',
  preferredProviders: [],
  excludedProviders: [],
  requireTools: [],
}

const VISIBILITIES: Visibility[] = ['private', 'team', 'network']
const REASSIGN: ReassignPolicy[] = ['never', 'same_capability', 'any_provider']
const STRATEGIES: CapabilityRouteStrategy[] = [
  'least_loaded',
  'round_robin',
  'most_trusted',
  'fastest',
  'highest_success_rate',
]
const FALLBACKS: FallbackPolicy[] = ['none', 'any_online', 'escalate_to_owner']

/** Non-negative whole number, or `fallback` for anything else */
function wholeNumber(value: string, fallback: number): number {
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback
}

/**
 * The owner's policy and routing form; read-only for everyone else.
 * Saving rewrites the capability's file.
 */
export function CapabilityPolicyForm({
  definition,
  editable,
}: {
  definition: CapabilityDefinition
  editable: boolean
}) {
  const { t } = useTranslation()
  const save = useSaveCapability()
  const [policy, setPolicy] = useState<CapabilityPolicy>(
    definition.policy ?? DEFAULT_POLICY
  )
  const [routing, setRouting] = useState<RoutingPolicy>(
    definition.routing ?? DEFAULT_ROUTING
  )
  const disabled = !editable || save.isPending

  const patchPolicy = (patch: Partial<CapabilityPolicy>) =>
    setPolicy(current => ({ ...current, ...patch }))
  const patchRouting = (patch: Partial<RoutingPolicy>) =>
    setRouting(current => ({ ...current, ...patch }))

  const onSave = () =>
    save.mutate(
      { ...definition, policy, routing },
      {
        onSuccess: () => toast.success(t('capabilities.policy.saved')),
        onError: error =>
          toast.error(t('capabilities.saveFailed', { error: rawError(error) })),
      }
    )

  return (
    <form
      className="flex max-w-xl flex-col gap-4"
      onSubmit={event => {
        event.preventDefault()
        onSave()
      }}
    >
      {!editable && (
        <p className="text-sm text-muted-foreground">
          {t('capabilities.policy.readOnly')}
        </p>
      )}
      <Row id="policy-visibility" label={t('capabilities.policy.visibility')}>
        <NativeSelect
          id="policy-visibility"
          value={policy.visibility}
          disabled={disabled}
          onChange={event =>
            patchPolicy({ visibility: event.target.value as Visibility })
          }
        >
          {VISIBILITIES.map(value => (
            <NativeSelectOption key={value} value={value}>
              {t(`capabilities.visibility.${value}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Row>
      <Row
        id="policy-per-provider"
        label={t('capabilities.policy.maxPerProvider')}
      >
        <Input
          id="policy-per-provider"
          type="number"
          min={1}
          value={policy.maxConcurrentPerProvider}
          disabled={disabled}
          onChange={event =>
            patchPolicy({
              maxConcurrentPerProvider: Math.max(
                1,
                wholeNumber(event.target.value, 1)
              ),
            })
          }
        />
      </Row>
      <Row
        id="policy-total"
        label={t('capabilities.policy.totalMax')}
        hint={t('capabilities.policy.zeroUnlimited')}
      >
        <Input
          id="policy-total"
          type="number"
          min={0}
          value={policy.totalMaxConcurrent}
          disabled={disabled}
          onChange={event =>
            patchPolicy({
              totalMaxConcurrent: wholeNumber(event.target.value, 0),
            })
          }
        />
      </Row>
      <Row
        id="policy-sla"
        label={t('capabilities.policy.sla')}
        hint={t('capabilities.policy.zeroNone')}
      >
        <Input
          id="policy-sla"
          type="number"
          min={0}
          value={policy.slaSeconds}
          disabled={disabled}
          onChange={event =>
            patchPolicy({ slaSeconds: wholeNumber(event.target.value, 0) })
          }
        />
      </Row>
      <div className="flex items-center justify-between gap-4">
        <Label htmlFor="policy-approval">
          {t('capabilities.policy.approvalRequired')}
        </Label>
        <Switch
          id="policy-approval"
          checked={policy.approvalRequired}
          disabled={disabled}
          onCheckedChange={approvalRequired =>
            patchPolicy({ approvalRequired })
          }
        />
      </div>
      <Row id="policy-reassign" label={t('capabilities.policy.autoReassign')}>
        <NativeSelect
          id="policy-reassign"
          value={policy.autoReassign}
          disabled={disabled}
          onChange={event =>
            patchPolicy({ autoReassign: event.target.value as ReassignPolicy })
          }
        >
          {REASSIGN.map(value => (
            <NativeSelectOption key={value} value={value}>
              {t(`capabilities.reassign.${value}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Row>
      <Row id="routing-strategy" label={t('capabilities.policy.strategy')}>
        <NativeSelect
          id="routing-strategy"
          value={routing.strategy}
          disabled={disabled}
          onChange={event =>
            patchRouting({
              strategy: event.target.value as CapabilityRouteStrategy,
            })
          }
        >
          {STRATEGIES.map(value => (
            <NativeSelectOption key={value} value={value}>
              {t(`capabilities.strategy.${value}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Row>
      <Row id="routing-fallback" label={t('capabilities.policy.fallback')}>
        <NativeSelect
          id="routing-fallback"
          value={routing.fallback}
          disabled={disabled}
          onChange={event =>
            patchRouting({ fallback: event.target.value as FallbackPolicy })
          }
        >
          {FALLBACKS.map(value => (
            <NativeSelectOption key={value} value={value}>
              {t(`capabilities.fallback.${value}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Row>
      {editable && (
        <div>
          <Button type="submit" disabled={save.isPending}>
            {save.isPending && <Spinner />}
            {t('capabilities.policy.save')}
          </Button>
        </div>
      )}
    </form>
  )
}

function Row({
  id,
  label,
  hint,
  children,
}: {
  id: string
  label: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-[14rem_1fr] sm:items-center">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex flex-col gap-1">
        {children}
        {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
      </div>
    </div>
  )
}
