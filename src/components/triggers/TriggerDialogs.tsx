import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { KeyRound } from 'lucide-react'
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
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { useTargetName } from '@/hooks/useTriggers'
import type { Trigger } from '@/lib/tauri-bindings'
import {
  deleteTrigger,
  rotateTriggerSecret,
  testFireTrigger,
} from '@/lib/trigger-actions'

/** `undefined` if the text isn't JSON */
function parseJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch {
    return undefined
  }
}

/**
 * Test…: a payload to send and a plain warning that it really runs the
 * target. Nothing fires until Run test.
 */
export function TestTriggerDialog({
  trigger,
  open,
  onOpenChange,
}: {
  trigger: Trigger
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const target = useTargetName(trigger)
  const [payload, setPayload] = useState('{\n  "test": true\n}')
  const parsed = parseJson(payload)

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('triggers.test.title', { name: trigger.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t(`triggers.test.warning.${trigger.targetType}`, { target })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="flex flex-col gap-2">
          <Label htmlFor={`test-payload-${trigger.id}`}>
            {t('triggers.testPayload')}
          </Label>
          <Textarea
            id={`test-payload-${trigger.id}`}
            dir="ltr"
            rows={5}
            className="font-mono text-xs"
            value={payload}
            aria-invalid={parsed === undefined}
            onChange={event => setPayload(event.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            {parsed === undefined
              ? t('triggers.invalidJson')
              : t('triggers.testPayloadHint')}
          </p>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('triggers.dialog.cancel')}</AlertDialogCancel>
          <AlertDialogAction
            disabled={parsed === undefined}
            onClick={() => void testFireTrigger(trigger.id, parsed)}
          >
            {t('triggers.test.run')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Generate a new secret…, after saying that callers using the old one break */
export function RotateSecretButton({
  trigger,
  onSecret,
}: {
  trigger: Trigger
  onSecret: (secret: string) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        className="self-start"
        onClick={() => setOpen(true)}
      >
        <KeyRound />
        {t('triggers.rotateSecret')}
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('triggers.rotate.title')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('triggers.rotate.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('triggers.dialog.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                void rotateTriggerSecret(trigger.id).then(secret => {
                  if (secret) onSecret(secret)
                })
              }
            >
              {t('triggers.rotate.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

/** Delete…, confirmed with the trigger's name */
export function DeleteTriggerDialog({
  trigger,
  open,
  onOpenChange,
}: {
  trigger: Trigger
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('triggers.deleteTitle', { name: trigger.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t('triggers.deleteDescription')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('triggers.dialog.cancel')}</AlertDialogCancel>
          <AlertDialogAction onClick={() => void deleteTrigger(trigger.id)}>
            {t('triggers.delete')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
