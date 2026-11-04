import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, Copy, KeyRound } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { webhookCurl } from '@/components/automations/automations-model'
import { useWebOrigin } from '@/hooks/useTriggers'
import { endpointPath } from '@/lib/trigger-form'
import type {
  FireRunStatus,
  Trigger,
  TriggerFire,
  TriggerFireStatus,
} from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'

const FIRE_STATUS_CLASSES: Record<TriggerFireStatus, string> = {
  queued: 'border-success/30 bg-success/10 text-success',
  rateLimited: 'border-warning/30 bg-warning/10 text-warning',
  error: 'border-destructive/30 bg-destructive/10 text-destructive',
  skipped: 'border-border bg-muted text-muted-foreground',
}

const RUN_STATUS_CLASSES: Record<FireRunStatus, string> = {
  queued: 'border-border bg-muted text-muted-foreground',
  running: 'border-info/30 bg-info/10 text-info',
  awaitingApproval: 'border-warning/30 bg-warning/10 text-warning',
  waiting: 'border-warning/30 bg-warning/10 text-warning',
  paused: 'border-border bg-muted text-muted-foreground',
  completed: 'border-success/30 bg-success/10 text-success',
  failed: 'border-destructive/30 bg-destructive/10 text-destructive',
  cancelled: 'border-border bg-muted text-muted-foreground',
}

/**
 * Where a fire got to: its run's status once it dispatched one
 * ("Completed", "Failed"), else the fire's own (rate limited, error,
 * skipped)
 */
export function FireStatusBadge({
  fire,
}: {
  fire: Pick<TriggerFire, 'status' | 'runStatus'>
}) {
  const { t } = useTranslation()
  if (fire.status === 'queued' && fire.runStatus) {
    return (
      <Badge variant="outline" className={RUN_STATUS_CLASSES[fire.runStatus]}>
        {t(`triggers.runStatus.${fire.runStatus}`)}
      </Badge>
    )
  }
  return (
    <Badge variant="outline" className={FIRE_STATUS_CLASSES[fire.status]}>
      {t(`triggers.fireStatus.${fire.status}`)}
    </Badge>
  )
}

export function CopyButton({
  value,
  label,
  disabled,
}: {
  value: string
  label: string
  disabled?: boolean
}) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="ghost"
      disabled={disabled}
      aria-label={label}
      title={copied ? t('triggers.copied') : label}
      onClick={() => {
        void navigator.clipboard.writeText(value).then(() => {
          setCopied(true)
          setTimeout(() => setCopied(false), 1500)
        })
      }}
    >
      {copied ? <Check /> : <Copy />}
    </Button>
  )
}

function CodeLine({
  value,
  copyLabel,
  className,
  disabled,
  multiline,
}: {
  value: string
  copyLabel: string
  className?: string
  /** Shows `value` but can't be copied yet */
  disabled?: boolean
  multiline?: boolean
}) {
  return (
    <div
      className={cn(
        'flex items-start gap-1 rounded-md border bg-muted/40 ps-2',
        className
      )}
    >
      <code
        dir="ltr"
        className={cn(
          'min-w-0 flex-1 py-1.5 font-mono text-xs',
          multiline ? 'whitespace-pre-wrap break-all' : 'truncate'
        )}
      >
        {value}
      </code>
      <CopyButton value={value} label={copyLabel} disabled={disabled} />
    </div>
  )
}

/**
 * Where a webhook/device trigger receives requests, and its secret: the
 * secret only right after it was generated (it's never shown again).
 */
export function TriggerEndpoint({
  trigger,
  secret,
}: {
  trigger: Trigger
  secret: string | null
}) {
  const { t } = useTranslation()
  const { data: origin } = useWebOrigin()
  const path = endpointPath(trigger)
  if (!path) return null
  // Until the local server answers, a relative path would copy wrong
  const url = origin ? `${origin}${path}` : null

  return (
    <div className="flex flex-col gap-2 text-sm">
      <p className="text-xs font-medium text-muted-foreground">
        {t('triggers.endpoint.url')}
      </p>
      <CodeLine
        value={url ?? t('triggers.endpoint.starting')}
        copyLabel={t('triggers.endpoint.copyUrl')}
        disabled={url === null}
      />
      <p className="text-xs text-muted-foreground">
        {t(
          trigger.sourceType === 'device'
            ? 'triggers.endpoint.deviceHelp'
            : 'triggers.endpoint.webhookHelp'
        )}
      </p>
      {trigger.sourceType === 'webhook' && url && (
        <>
          <p className="text-xs font-medium text-muted-foreground">
            {t('triggers.endpoint.tryIt')}
          </p>
          <CodeLine
            value={webhookCurl(url)}
            copyLabel={t('triggers.endpoint.copyCurl')}
            multiline
          />
        </>
      )}
      {secret ? (
        <div className="flex flex-col gap-1.5 rounded-md border border-warning/40 bg-warning/5 p-3">
          <p className="flex items-center gap-1.5 text-xs font-medium text-warning [&_svg]:size-3.5">
            <KeyRound aria-hidden />
            {t('triggers.endpoint.secretOnce')}
          </p>
          <CodeLine
            value={secret}
            copyLabel={t('triggers.endpoint.copySecret')}
            className="bg-background"
          />
          <p className="text-xs text-muted-foreground">
            {t('triggers.endpoint.header')}{' '}
            <code dir="ltr" className="font-mono">
              X-Trigger-Secret
            </code>
          </p>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          {t('triggers.endpoint.secretHidden')}
        </p>
      )}
    </div>
  )
}
