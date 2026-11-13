import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import type { ConsentStatus } from '@/lib/bindings'
import { marketplaceQueryKeys } from '@/hooks/useMarketplace'
import { grantConsent } from '@/lib/marketplace-actions'

const CATEGORY_LABELS = {
  tools: 'marketplace.consent.tools',
  network: 'marketplace.consent.network',
  aws: 'marketplace.consent.aws',
  pilots: 'marketplace.consent.pilots',
  connectors: 'marketplace.consent.connectors',
  folders: 'marketplace.consent.folders',
  delegation: 'marketplace.consent.delegation',
} as const

const isCategory = (key: string): key is keyof typeof CATEGORY_LABELS =>
  key in CATEGORY_LABELS

interface ConsentDialogProps {
  status: ConsentStatus | null
  onOpenChange: (open: boolean) => void
}

/**
 * First run of a marketplace agent: everything its skill.md requests, one
 * category at a time. Each must be ticked before it may run.
 */
export function ConsentDialog({ status, onOpenChange }: ConsentDialogProps) {
  return (
    <Dialog open={status !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        {status && (
          <ConsentForm
            key={status.agentId}
            status={status}
            onDone={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function ConsentForm({
  status,
  onDone,
}: {
  status: ConsentStatus
  onDone: () => void
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [checked, setChecked] = useState<string[]>(
    status.categories.filter(c => c.granted).map(c => c.key)
  )
  const [saving, setSaving] = useState(false)
  const allChecked = status.categories.every(c => checked.includes(c.key))

  const allow = async () => {
    setSaving(true)
    const updated = await grantConsent(
      status.agentId,
      status.categories
        .filter(category => checked.includes(category.key))
        .map(({ key, items }) => ({ key, items }))
    )
    setSaving(false)
    if (!updated) return
    queryClient.setQueryData(
      marketplaceQueryKeys.consent(status.agentId),
      updated
    )
    if (updated.satisfied) onDone()
  }

  return (
    <div className="grid gap-4">
      <DialogHeader>
        <DialogTitle>
          {t('marketplace.consent.title', { name: status.agentName })}
        </DialogTitle>
        <DialogDescription>
          {t('marketplace.consent.description')}
        </DialogDescription>
      </DialogHeader>
      <ul className="grid gap-3">
        {status.categories.map(category => {
          const id = `consent-${category.key}`
          const items =
            category.key === 'delegation'
              ? t('marketplace.consent.delegationItems')
              : category.items.includes('*')
                ? t('marketplace.consent.allTools')
                : category.items.join(', ')
          return (
            <li key={category.key} className="flex items-start gap-3">
              <Checkbox
                id={id}
                checked={checked.includes(category.key)}
                onCheckedChange={value =>
                  setChecked(current =>
                    value === true
                      ? [...current, category.key]
                      : current.filter(key => key !== category.key)
                  )
                }
              />
              <Label htmlFor={id} className="grid gap-1 font-normal">
                <span className="font-medium">
                  {isCategory(category.key)
                    ? t(CATEGORY_LABELS[category.key])
                    : category.key}
                </span>
                <span className="font-mono text-xs break-all text-muted-foreground">
                  {items}
                </span>
              </Label>
            </li>
          )
        })}
      </ul>
      <DialogFooter>
        <Button disabled={!allChecked || saving} onClick={() => void allow()}>
          {saving ? <Spinner /> : <ShieldCheck />}
          {t('marketplace.consent.allow')}
        </Button>
      </DialogFooter>
    </div>
  )
}
