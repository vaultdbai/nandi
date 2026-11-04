import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Pencil, Trash2, Users } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { parseList, useGatewayUsage } from '@/hooks/useGateway'
import type { CallerPolicy } from '@/lib/bindings'

export interface GatewayCallersTableProps {
  callers: CallerPolicy[]
  onSave: (policy: CallerPolicy) => void
  onRemove: (callerInstance: string) => void
  busy?: boolean
  /** A caller's display name from the directory (`null`: unknown) */
  nameOf?: (callerInstance: string) => string | null
}

/**
 * Per-caller policies (rows appear on a caller's first request): enable
 * or disable, edit the allowed capabilities and limits, remove. Shows
 * this month's use against the budget.
 */
export function GatewayCallersTable({
  callers,
  onSave,
  onRemove,
  busy = false,
  nameOf,
}: GatewayCallersTableProps) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState<string | null>(null)
  // A name, never the device ID (it's under Details)
  const callerName = (caller: CallerPolicy) =>
    nameOf?.(caller.callerInstance) ||
    caller.callerOwner ||
    t('network.unnamed')

  if (callers.length === 0) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Users />
          </EmptyMedia>
          <EmptyTitle>{t('gateway.callers.empty.title')}</EmptyTitle>
          <EmptyDescription>
            {t('gateway.callers.empty.description')}
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm" data-testid="gateway-callers">
        <thead className="text-muted-foreground">
          <tr className="border-b text-start">
            <th className="py-2 text-start font-medium">
              {t('gateway.callers.caller')}
            </th>
            <th className="py-2 text-start font-medium">
              {t('gateway.callers.allowed')}
            </th>
            <th className="py-2 text-start font-medium">
              {t('gateway.callers.limits')}
            </th>
            <th className="py-2 text-start font-medium">
              {t('gateway.callers.usage')}
            </th>
            <th className="py-2 text-start font-medium">
              {t('gateway.callers.enabled')}
            </th>
            <th className="py-2" />
          </tr>
        </thead>
        <tbody>
          {callers.map(caller =>
            editing === caller.callerInstance ? (
              <EditRow
                key={caller.callerInstance}
                caller={caller}
                name={callerName(caller)}
                busy={busy}
                onCancel={() => setEditing(null)}
                onSave={policy => {
                  onSave(policy)
                  setEditing(null)
                }}
              />
            ) : (
              <tr
                key={caller.callerInstance}
                className="border-b last:border-0"
                data-testid={`gateway-caller-${caller.callerInstance}`}
              >
                <td className="py-2">
                  <div className="font-medium">{callerName(caller)}</div>
                  <details className="text-xs text-muted-foreground">
                    <summary className="cursor-pointer">
                      {t('network.details.title')}
                    </summary>
                    <code dir="ltr" className="font-mono break-all">
                      {caller.callerInstance}
                    </code>
                  </details>
                </td>
                <td className="py-2">
                  {caller.allowedCapabilities.length > 0
                    ? caller.allowedCapabilities.join(', ')
                    : t('gateway.callers.anyExposed')}
                </td>
                <td className="py-2">
                  {t('gateway.callers.limitsValue', {
                    max: caller.maxConcurrent,
                    budget:
                      caller.monthlyTaskBudget ??
                      t('gateway.callers.unlimited'),
                  })}
                </td>
                <td className="py-2">
                  <UsageCell caller={caller} />
                </td>
                <td className="py-2">
                  <Switch
                    checked={!caller.disabled}
                    disabled={busy}
                    aria-label={t('gateway.callers.toggle', {
                      caller: callerName(caller),
                    })}
                    onCheckedChange={enabled =>
                      onSave({ ...caller, disabled: !enabled })
                    }
                  />
                </td>
                <td className="py-2 text-end">
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={t('gateway.callers.edit', {
                      caller: callerName(caller),
                    })}
                    onClick={() => setEditing(caller.callerInstance)}
                  >
                    <Pencil />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    disabled={busy}
                    aria-label={t('gateway.callers.remove', {
                      caller: callerName(caller),
                    })}
                    onClick={() => onRemove(caller.callerInstance)}
                  >
                    <Trash2 />
                  </Button>
                </td>
              </tr>
            )
          )}
        </tbody>
      </table>
    </div>
  )
}

function UsageCell({ caller }: { caller: CallerPolicy }) {
  const { t } = useTranslation()
  const usage = useGatewayUsage(caller.callerInstance)
  if (!usage.data) return <span className="text-muted-foreground">–</span>
  return (
    <span>
      {usage.data.monthlyTaskBudget === null
        ? t('gateway.callers.usageCount', { count: usage.data.taskCount })
        : t('gateway.callers.usageOfBudget', {
            count: usage.data.taskCount,
            budget: usage.data.monthlyTaskBudget,
          })}
    </span>
  )
}

function EditRow({
  caller,
  name,
  busy,
  onSave,
  onCancel,
}: {
  caller: CallerPolicy
  name: string
  busy: boolean
  onSave: (policy: CallerPolicy) => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const [allowed, setAllowed] = useState(caller.allowedCapabilities.join(', '))
  const [max, setMax] = useState(String(caller.maxConcurrent))
  const [budget, setBudget] = useState(
    caller.monthlyTaskBudget === null ? '' : String(caller.monthlyTaskBudget)
  )
  const parsedMax = Number.parseInt(max, 10)
  const parsedBudget = budget.trim() === '' ? null : Number.parseInt(budget, 10)
  const valid =
    Number.isFinite(parsedMax) &&
    parsedMax >= 1 &&
    (parsedBudget === null ||
      (Number.isFinite(parsedBudget) && parsedBudget >= 0))

  return (
    <tr
      className="border-b"
      data-testid={`gateway-caller-edit-${caller.callerInstance}`}
    >
      <td className="py-2 font-medium">{name}</td>
      <td className="py-2 pe-2">
        <Input
          dir="ltr"
          value={allowed}
          placeholder={t('gateway.callers.anyExposed')}
          aria-label={t('gateway.callers.allowed')}
          onChange={e => setAllowed(e.target.value)}
        />
      </td>
      <td className="grid gap-1 py-2 pe-2">
        <Input
          type="number"
          min={1}
          value={max}
          aria-label={t('gateway.callers.maxConcurrent')}
          onChange={e => setMax(e.target.value)}
        />
        <Input
          type="number"
          min={0}
          value={budget}
          placeholder={t('gateway.callers.unlimited')}
          aria-label={t('gateway.callers.budget')}
          onChange={e => setBudget(e.target.value)}
        />
      </td>
      <td className="py-2" />
      <td className="py-2" />
      <td className="py-2 text-end">
        <div className="flex justify-end gap-1">
          <Button variant="outline" size="sm" onClick={onCancel}>
            {t('gateway.callers.cancel')}
          </Button>
          <Button
            size="sm"
            disabled={!valid || busy}
            onClick={() =>
              onSave({
                ...caller,
                allowedCapabilities: parseList(allowed),
                maxConcurrent: parsedMax,
                monthlyTaskBudget: parsedBudget,
              })
            }
          >
            {t('gateway.callers.save')}
          </Button>
        </div>
      </td>
    </tr>
  )
}
