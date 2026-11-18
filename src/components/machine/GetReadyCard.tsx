import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, CircleAlert, ExternalLink, ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import {
  useComputerReady,
  useGetComputerReady,
  useMachineSettings,
  useSetStartDockerOnOpen,
  useStartDocker,
} from '@/hooks/useMachine'
import type { MachineReport, ReadyStepProgress } from '@/lib/bindings'
import { client } from '@/lib/client'
import {
  adminKey,
  asksStartOnOpen,
  handoverKey,
  readyView,
  showsLicence,
  stepKey,
  titleKey,
} from './ready-model'

/**
 * "Get your computer ready?": one card, one go-ahead. It lists the steps
 * from the core's plan, says the OS asks for admin rights for each
 * installer, and shows Docker's licence line. Go ahead runs the steps in
 * the core, checking again after each; a failed step shows what Nandi saw
 * and the page to do it by hand. Not now keeps the checklist.
 */
export function GetReadyCard({ report }: { report: MachineReport }) {
  const { t } = useTranslation()
  const [dismissed, setDismissed] = useState(false)
  const status = useComputerReady()
  const goAhead = useGetComputerReady()
  const view = readyView(report, status.data, dismissed)
  if (view.kind === 'hidden') return null

  if (view.kind === 'ask')
    return (
      <Frame testId="get-ready-ask">
        <p className="text-sm font-medium">{t(titleKey(view.plan))}</p>
        <StepList
          steps={view.plan.map(step => ({ step, status: 'waiting' }))}
        />
        <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <ShieldCheck aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          {t(adminKey(report.os))}
        </p>
        {showsLicence(view.plan) && (
          <p className="text-xs text-muted-foreground">
            {t('machine.ready.licence')}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={goAhead.isPending}
            onClick={() => goAhead.mutate(view.plan)}
          >
            {goAhead.isPending && <Spinner />}
            {t('machine.ready.goAhead')}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setDismissed(true)}>
            {t('machine.ready.notNow')}
          </Button>
        </div>
      </Frame>
    )

  const { progress } = view
  return (
    <Frame testId={`get-ready-${view.kind}`}>
      <p className="text-sm font-medium">
        {t(`machine.ready.status.${view.kind}`)}
      </p>
      <StepList steps={progress.steps} />
      {view.kind === 'handedOver' && progress.handover && (
        <p className="text-sm">{t(handoverKey(progress.handover))}</p>
      )}
      {view.kind === 'failed' && progress.failure && (
        <p className="text-xs break-words text-muted-foreground">
          {progress.failure.detail}
        </p>
      )}
      {progress.log.length > 0 && view.kind !== 'done' && (
        <pre
          dir="ltr"
          className="max-h-32 overflow-auto rounded-md bg-muted px-2 py-1 font-mono text-xs whitespace-pre-wrap"
        >
          {progress.log.slice(-8).join('\n')}
        </pre>
      )}
      <div className="flex flex-wrap gap-2">
        {view.kind === 'failed' && progress.failure && (
          <LinkButton url={progress.failure.link} />
        )}
        {view.kind === 'handedOver' && progress.handoverLink && (
          <LinkButton url={progress.handoverLink} />
        )}
        {view.kind === 'failed' && (
          <Button
            size="sm"
            variant="outline"
            disabled={goAhead.isPending || report.plan.length === 0}
            onClick={() => goAhead.mutate(report.plan)}
          >
            {t('machine.ready.tryAgain')}
          </Button>
        )}
        {view.kind === 'done' && (
          <Button size="sm" variant="ghost" onClick={() => setDismissed(true)}>
            {t('machine.ready.close')}
          </Button>
        )}
      </div>
    </Frame>
  )
}

function Frame({ testId, children }: { testId: string; children: ReactNode }) {
  return (
    <section
      data-testid={testId}
      className="flex flex-col gap-2 rounded-md border bg-muted/30 p-3"
    >
      {children}
    </section>
  )
}

function StepList({ steps }: { steps: ReadyStepProgress[] }) {
  const { t } = useTranslation()
  return (
    <ol className="flex list-decimal flex-col gap-1 ps-5 text-sm">
      {steps.map(({ step, status }) => (
        <li key={step} data-status={status}>
          <span className="inline-flex items-center gap-1.5">
            {t(stepKey(step))}
            <StepIcon status={status} />
          </span>
        </li>
      ))}
    </ol>
  )
}

function StepIcon({ status }: { status: ReadyStepProgress['status'] }) {
  if (status === 'running') return <Spinner className="size-3.5" />
  if (status === 'done') return <Check className="size-3.5 text-success" />
  if (status === 'failed')
    return <CircleAlert className="size-3.5 text-warning" />
  return null
}

function LinkButton({ url }: { url: string }) {
  const { t } = useTranslation()
  return (
    <Button
      size="sm"
      variant="outline"
      onClick={() => void client.openExternal(url)}
    >
      <ExternalLink />
      {t('machine.ready.doItYourself')}
    </Button>
  )
}

/**
 * "Start Docker automatically when Nandi opens?": asked once, the first
 * time the check finds Docker stopped. Yes also starts it now.
 */
export function StartOnOpenQuestion({ report }: { report: MachineReport }) {
  const { t } = useTranslation()
  const settings = useMachineSettings()
  const answer = useSetStartDockerOnOpen()
  const start = useStartDocker()
  if (!asksStartOnOpen(report, settings.data)) return null
  return (
    <Frame testId="start-on-open-question">
      <p className="text-sm">{t('machine.ready.startOnOpen')}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          disabled={answer.isPending}
          onClick={() => {
            answer.mutate(true)
            start.mutate()
          }}
        >
          {t('machine.ready.startOnOpenYes')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={answer.isPending}
          onClick={() => answer.mutate(false)}
        >
          {t('machine.ready.startOnOpenNo')}
        </Button>
      </div>
    </Frame>
  )
}
