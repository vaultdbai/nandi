import { useTranslation } from 'react-i18next'
import {
  Check,
  CircleAlert,
  Copy,
  ExternalLink,
  HelpCircle,
} from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useStartDocker } from '@/hooks/useMachine'
import type { MachineAction, MachineCheck, MachineReport } from '@/lib/bindings'
import { client } from '@/lib/client'
import { cn } from '@/lib/utils'
import {
  actionKey,
  rowActions,
  STATE_TONE,
  stepsFor,
  whyKey,
  type MachineStep,
} from './machine-model'

const ICONS = {
  ok: <Check className="text-success" />,
  warning: <CircleAlert className="text-warning" />,
  muted: <HelpCircle className="text-muted-foreground" />,
}

/**
 * The "Get your computer ready" checklist: one row per check, and for each
 * one that isn't ready, why it matters, numbered steps for this computer,
 * and its actions. Used by Settings › This computer and the onboarding step.
 */
export function MachineChecklist({
  report,
  className,
}: {
  report: MachineReport | undefined
  className?: string
}) {
  if (!report)
    return (
      <div className="flex flex-col gap-2" data-testid="machine-loading">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-10 w-full" />
      </div>
    )
  return (
    <ol
      className={cn('flex flex-col divide-y rounded-md border', className)}
      data-testid="machine-checklist"
    >
      {report.checks.map(check => (
        <CheckRow key={check.id} check={check} report={report} />
      ))}
    </ol>
  )
}

function CheckRow({
  check,
  report,
}: {
  check: MachineCheck
  report: MachineReport
}) {
  const { t } = useTranslation()
  const tone = STATE_TONE[check.state]
  const steps = stepsFor(check, report)
  const actions = rowActions(check)
  return (
    <li
      className="flex items-start gap-3 px-3 py-2.5"
      data-testid={`machine-${check.id}`}
      data-state={check.state}
    >
      <span aria-hidden className="mt-0.5 shrink-0 [&_svg]:size-4">
        {ICONS[tone]}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-sm font-medium">
            {t(`machine.check.${check.id}`)}
          </span>
          <span
            className={cn(
              'text-xs',
              tone === 'ok' ? 'text-success' : 'text-muted-foreground'
            )}
          >
            {t(`machine.state.${check.state}`)}
          </span>
        </div>
        <p
          className="text-xs break-words text-muted-foreground"
          title={t('machine.detailTitle')}
        >
          {check.detail}
        </p>
        {check.state !== 'ok' && (
          <>
            <p className="text-sm">{t(whyKey(check.id))}</p>
            {steps.length > 0 && <Steps steps={steps} />}
            {/* Phase 2: approved "Install for me" / fix actions join these */}
            {actions.length > 0 && <MachineActions actions={actions} />}
          </>
        )}
      </div>
    </li>
  )
}

function Steps({ steps }: { steps: MachineStep[] }) {
  const { t } = useTranslation()
  return (
    <ol className="flex list-decimal flex-col gap-1 ps-5 text-sm">
      {steps.map(step => (
        <li key={step.key}>
          <span>{t(`machine.step.${step.key}`, step.params)}</span>
          {step.command && <Command command={step.command} />}
        </li>
      ))}
    </ol>
  )
}

function Command({ command }: { command: string }) {
  const { t } = useTranslation()
  return (
    <div className="mt-1 flex items-center gap-1 rounded-md bg-muted px-2 py-1">
      <code dir="ltr" className="min-w-0 flex-1 font-mono text-xs break-all">
        {command}
      </code>
      <Button
        size="icon-sm"
        variant="ghost"
        className="size-6"
        aria-label={t('machine.copyCommand')}
        title={t('machine.copyCommand')}
        onClick={() => {
          void navigator.clipboard?.writeText(command)
          toast.success(t('machine.copied'))
        }}
      >
        <Copy />
      </Button>
    </div>
  )
}

/**
 * A check's buttons. Phase 1: Start Docker and official links. Phase 2's
 * approved installs and fixes are new `MachineAction` kinds rendered here.
 */
export function MachineActions({ actions }: { actions: MachineAction[] }) {
  const { t } = useTranslation()
  const start = useStartDocker()
  return (
    <div className="flex flex-wrap gap-2">
      {actions.map(action =>
        action.kind === 'openLink' ? (
          <Button
            key={action.url}
            size="sm"
            variant="outline"
            onClick={() => void client.openExternal(action.url)}
          >
            <ExternalLink />
            {t(actionKey(action))}
          </Button>
        ) : action.kind === 'startDocker' ? (
          <Button
            key="startDocker"
            size="sm"
            disabled={start.isPending}
            onClick={() => start.mutate()}
          >
            {start.isPending && <Spinner />}
            {t(actionKey(action))}
          </Button>
        ) : null
      )}
    </div>
  )
}
