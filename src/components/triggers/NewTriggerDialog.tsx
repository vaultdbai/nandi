import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { createTrigger } from '@/lib/trigger-actions'
import { useTriggerFormProblems } from '@/hooks/useTriggers'
import {
  emptyForm,
  SOURCE_TYPES,
  toInput,
  type TriggerFormState,
} from '@/lib/trigger-form'
import type { SavedTrigger, TriggerSourceType } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { SOURCE_ICONS } from './source-icons'
import { TriggerEndpoint } from './trigger-ui'
import { TriggerForm } from './TriggerForm'

/**
 * Creates a trigger: a tab per source type, then the shared form. A
 * webhook/device trigger's secret is shown here once, after saving.
 */
export function NewTriggerDialog({
  open,
  onOpenChange,
  initial,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Prefills the form, e.g. the workflow "Add a trigger" came from */
  initial?: Partial<TriggerFormState>
}) {
  const { t } = useTranslation()
  const fresh = () => ({ ...emptyForm(), ...initial })
  const [form, setForm] = useState<TriggerFormState>(fresh)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState<SavedTrigger | null>(null)

  const close = (next: boolean) => {
    onOpenChange(next)
    if (!next) {
      setForm(fresh())
      setSaved(null)
    }
  }

  const finish = (trigger: SavedTrigger) => {
    close(false)
    useNavigationStore
      .getState()
      .navigate({ kind: 'trigger', triggerId: trigger.trigger.id })
  }

  const save = async () => {
    setSaving(true)
    const result = await createTrigger(toInput(form))
    setSaving(false)
    if (!result) return
    // Show the secret before leaving; it can't be retrieved later
    if (result.secret) setSaved(result)
    else finish(result)
  }

  const problems = useTriggerFormProblems(form)

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent className="flex max-h-[90vh] flex-col gap-4 sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {saved ? t('triggers.dialog.savedTitle') : t('triggers.new')}
          </DialogTitle>
          <DialogDescription>
            {saved
              ? t('triggers.dialog.savedDescription')
              : t('triggers.dialog.description')}
          </DialogDescription>
        </DialogHeader>

        {saved ? (
          <TriggerEndpoint trigger={saved.trigger} secret={saved.secret} />
        ) : (
          <>
            <Tabs
              value={form.sourceType}
              onValueChange={value =>
                setForm(current => ({
                  ...current,
                  sourceType: value as TriggerSourceType,
                }))
              }
            >
              <TabsList className="w-full">
                {SOURCE_TYPES.map(type => (
                  <TabsTrigger key={type} value={type} className="gap-1.5">
                    {SOURCE_ICONS[type]}
                    {t(`triggers.source.${type}`)}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            {/* Scrolls inside the dialog: a ScrollArea can't, since the
                dialog only has a max height */}
            <div className="min-h-0 flex-1 overflow-y-auto pe-3">
              <TriggerForm
                form={form}
                onChange={patch =>
                  setForm(current => ({ ...current, ...patch }))
                }
              />
            </div>
          </>
        )}

        <DialogFooter>
          {saved ? (
            <Button onClick={() => finish(saved)}>
              {t('triggers.dialog.done')}
            </Button>
          ) : (
            <>
              <Button variant="outline" onClick={() => close(false)}>
                {t('triggers.dialog.cancel')}
              </Button>
              <Button
                disabled={saving || problems.length > 0}
                onClick={() => void save()}
              >
                {t('triggers.dialog.create')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
