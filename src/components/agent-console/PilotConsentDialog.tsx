import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ShieldCheck } from 'lucide-react'
import { toast } from 'sonner'
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
import { grantPilotCapabilities } from '@/hooks/usePilots'
import type { PilotDefinition } from '@/lib/bindings'

interface PilotConsentDialogProps {
  /** The agent asking; `null` closes the dialog */
  agent: { id: string; name: string } | null
  pilot: PilotDefinition | null
  onOpenChange: (open: boolean) => void
  /** After every capability was granted */
  onGranted?: () => void
}

/**
 * Asks the user to grant an agent a pilot's capabilities. Every capability
 * the pilot exposes is listed and must be ticked: the core only routes an
 * agent to a pilot once all of them are granted (and its skill.md declares
 * the pilot).
 */
export function PilotConsentDialog({
  agent,
  pilot,
  onOpenChange,
  onGranted,
}: PilotConsentDialogProps) {
  const open = agent !== null && pilot !== null
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        {agent && pilot && (
          <ConsentForm
            key={`${agent.id}/${pilot.id}`}
            agent={agent}
            pilot={pilot}
            onDone={() => {
              onGranted?.()
              onOpenChange(false)
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function ConsentForm({
  agent,
  pilot,
  onDone,
}: {
  agent: { id: string; name: string }
  pilot: PilotDefinition
  onDone: () => void
}) {
  const { t } = useTranslation()
  const [checked, setChecked] = useState<string[]>([])
  const [saving, setSaving] = useState(false)
  const allChecked = pilot.capabilities.every(c => checked.includes(c))

  const allow = async () => {
    setSaving(true)
    const error = await grantPilotCapabilities(
      agent.id,
      pilot.id,
      pilot.capabilities
    )
    setSaving(false)
    if (error) {
      toast.error(t('pilots.consent.failed'), { description: error })
      return
    }
    toast.success(
      t('pilots.consent.granted', { agent: agent.name, pilot: pilot.name })
    )
    onDone()
  }

  return (
    <div className="grid gap-4">
      <DialogHeader>
        <DialogTitle>
          {t('pilots.consent.title', { agent: agent.name, pilot: pilot.name })}
        </DialogTitle>
        <DialogDescription>{t('pilots.consent.description')}</DialogDescription>
      </DialogHeader>
      <ul className="grid gap-3">
        {pilot.capabilities.map(capability => {
          const id = `pilot-consent-${capability}`
          return (
            <li key={capability} className="flex items-start gap-3">
              <Checkbox
                id={id}
                checked={checked.includes(capability)}
                onCheckedChange={value =>
                  setChecked(current =>
                    value === true
                      ? [...current, capability]
                      : current.filter(c => c !== capability)
                  )
                }
              />
              <Label htmlFor={id} className="grid gap-1 font-normal">
                <span className="font-medium">
                  {t(`pilots.capability.${capability}`, {
                    defaultValue: capability,
                  })}
                </span>
                <span className="font-mono text-xs text-muted-foreground">
                  {capability}
                </span>
              </Label>
            </li>
          )
        })}
      </ul>
      <DialogFooter>
        <Button disabled={!allChecked || saving} onClick={() => void allow()}>
          {saving ? <Spinner /> : <ShieldCheck />}
          {t('pilots.consent.allow')}
        </Button>
      </DialogFooter>
    </div>
  )
}
