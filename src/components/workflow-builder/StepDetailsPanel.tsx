import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import { Check, ExternalLink, TriangleAlert, X } from 'lucide-react'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { stepLabel } from '@/components/automations/automations-model'
import {
  StepApprovalButtons,
  MarkStepDoneButton,
} from '@/components/automations/StepActions'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Textarea } from '@/components/ui/textarea'
import { useAgents } from '@/hooks/useAgents'
import { useWorkflow } from '@/hooks/useSnapshots'
import type {
  GraphNode,
  Workflow,
  WorkflowRun,
  WorkflowStep,
} from '@/lib/bindings'
import { client } from '@/lib/client'
import { formatDuration } from '@/lib/format-time'
import { unwrapResult } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { saveWorkflowYaml } from './actions'
import {
  referencedSteps,
  STATUS_PILL_CLASSES,
  stepStatus,
  workflowGraphKey,
  workflowYamlWithStep,
} from './graph-model'

interface StepDetailsPanelProps {
  workflowId: string
  stepId: string | null
  /** The run the graph shows; its waiting step gets Approve / Mark done */
  run?: WorkflowRun
  onClose?: () => void
  /** After "Edit step" saves a new ID for the step */
  onStepRenamed?: (stepId: string) => void
}

/** The selected step: what it does, how it went, and its editor. */
export function StepDetailsPanel({
  workflowId,
  stepId,
  run,
  onClose,
  onStepRenamed,
}: StepDetailsPanelProps) {
  const { t } = useTranslation()
  // Shares the canvas's cache entry (kept live by its event subscription)
  const { data: graph } = useQuery({
    queryKey: workflowGraphKey(workflowId),
    queryFn: async () =>
      unwrapResult(await client.getWorkflowGraph(workflowId)),
  })
  const { data: workflow } = useWorkflow(workflowId)
  const node = graph?.nodes.find(n => n.id === stepId)
  const definition = workflow?.steps.find(step => step.id === stepId)

  return (
    <section
      aria-label={t('workflowBuilder.stepDetails')}
      className="flex h-full min-h-0 flex-col"
    >
      <header className="flex h-11 shrink-0 items-center justify-between border-b px-4">
        <h2 className="truncate text-sm font-semibold">
          {stepId
            ? t('automations.step.title', { step: stepLabel(stepId) })
            : t('workflowBuilder.stepDetails')}
        </h2>
        {onClose && (
          <Button
            size="icon"
            variant="ghost"
            className="size-7"
            aria-label={t('workflowBuilder.closeDetails')}
            onClick={onClose}
          >
            <X />
          </Button>
        )}
      </header>
      <ScrollArea className="min-h-0 flex-1">
        {node && definition && workflow ? (
          <StepDetails
            key={node.id}
            workflow={workflow}
            node={node}
            definition={definition}
            run={run}
            onStepRenamed={onStepRenamed}
          />
        ) : (
          <p className="p-4 text-sm text-muted-foreground">
            {t('workflowBuilder.selectStep')}
          </p>
        )}
      </ScrollArea>
    </section>
  )
}

function StepDetails({
  workflow,
  node,
  definition,
  run,
  onStepRenamed,
}: {
  workflow: Workflow
  node: GraphNode
  definition: WorkflowStep
  run?: WorkflowRun
  onStepRenamed?: (stepId: string) => void
}) {
  const { t, i18n } = useTranslation()
  const { data: agents } = useAgents()
  const status = stepStatus(node)
  const agentRunId = node.agentRunId
  const manual = definition.manual === true
  const agentKnown =
    manual || (agents ?? []).some(agent => agent.id === definition.agent)
  const runStep = run?.steps.find(step => step.stepId === node.id)
  const duration =
    node.startedAt !== null && node.endedAt !== null
      ? formatDuration(node.endedAt - node.startedAt)
      : status === 'running'
        ? t('workflowBuilder.inProgress')
        : '—'
  const references = referencedSteps(node.inputTemplate)

  return (
    <div className="flex flex-col gap-4 p-4">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
        <dt className="text-muted-foreground">
          {t('workflowBuilder.fields.agent')}
        </dt>
        <dd className="truncate text-end">
          {manual
            ? t('automations.step.byPerson')
            : (node.agentName ?? definition.agent)}
        </dd>
        <dt className="text-muted-foreground">
          {t('workflowBuilder.fields.status')}
        </dt>
        <dd className="text-end">
          <Badge variant="outline" className={STATUS_PILL_CLASSES[status]}>
            {t(`workflowBuilder.status.${status}`)}
          </Badge>
        </dd>
        <dt className="text-muted-foreground">
          {t('workflowBuilder.fields.started')}
        </dt>
        <dd className="text-end tabular-nums">
          {node.startedAt !== null
            ? new Date(node.startedAt).toLocaleTimeString(i18n.language)
            : '—'}
        </dd>
        <dt className="text-muted-foreground">
          {t('workflowBuilder.fields.duration')}
        </dt>
        <dd className="text-end tabular-nums">{duration}</dd>
      </dl>

      {!agentKnown && (
        <p
          role="alert"
          className="flex items-center gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-2 text-xs text-destructive"
        >
          <TriangleAlert aria-hidden className="size-4 shrink-0" />
          {t('automations.step.missingAgent')}
        </p>
      )}

      {run && runStep?.status === 'awaitingApproval' && (
        <StepApprovalButtons
          workflowId={workflow.id}
          runId={run.id}
          stepId={node.id}
        />
      )}
      {run && runStep?.status === 'awaitingInput' && (
        <MarkStepDoneButton
          workflowId={workflow.id}
          runId={run.id}
          stepId={node.id}
        />
      )}

      {agentRunId && (
        <Button
          size="sm"
          variant="outline"
          className="self-start"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'run-detail', runId: agentRunId })
          }
        >
          <ExternalLink />
          {t('workflowBuilder.viewRun')}
        </Button>
      )}

      {node.input !== null && (
        <KeyValue label={t('workflowBuilder.fields.sent')}>
          {node.input}
        </KeyValue>
      )}
      {node.output ? (
        <div className="rounded-md border p-3">
          <MarkdownContent content={node.output} />
        </div>
      ) : null}
      {node.error && (
        <p role="alert" className="text-sm text-destructive">
          {node.error}
        </p>
      )}

      {manual ? (
        <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          {t('automations.step.manualEditInStudio')}
        </p>
      ) : (
        <EditStep
          workflow={workflow}
          step={definition}
          active={run?.endedAt === null}
          onStepRenamed={onStepRenamed}
        />
      )}

      <Collapsible>
        <CollapsibleTrigger className="text-xs text-muted-foreground hover:text-foreground">
          {t('automations.details')}
        </CollapsibleTrigger>
        <CollapsibleContent className="flex flex-col gap-2 pt-2 text-xs">
          <KeyValue label={t('automations.step.id')} mono>
            {node.id}
          </KeyValue>
          {references.length > 0 && (
            <KeyValue label={t('workflowBuilder.fields.usesOutputOf')} mono>
              {references.join(', ')}
            </KeyValue>
          )}
          <KeyValue label={t('workflowBuilder.fields.onFailure')}>
            {node.continueOnFailure
              ? t('workflowBuilder.onFailure.continue')
              : t('workflowBuilder.onFailure.fail')}
          </KeyValue>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

function KeyValue({
  label,
  mono,
  children,
}: {
  label: string
  mono?: boolean
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-0.5 rounded-md border px-3 py-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span
        dir="auto"
        className={cn(
          'text-sm break-words whitespace-pre-wrap',
          mono && 'font-mono text-xs'
        )}
      >
        {children}
      </span>
    </div>
  )
}

/** Always available on the definition, whatever the last run did */
function EditStep({
  workflow,
  step,
  active,
  onStepRenamed,
}: {
  workflow: Workflow
  step: WorkflowStep
  /** A run is going: the change applies to the next one */
  active: boolean
  onStepRenamed?: (stepId: string) => void
}) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  const [agent, setAgent] = useState(step.agent)
  const [input, setInput] = useState(step.input)
  const [askFirst, setAskFirst] = useState(step.requiresApproval === true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const known = agents.some(a => a.id === agent)
  const changed =
    agent !== step.agent ||
    input !== step.input ||
    askFirst !== (step.requiresApproval === true)

  const save = async () => {
    setSaving(true)
    setError(null)
    const yaml = workflowYamlWithStep(workflow, step.id, {
      id: step.id,
      agent,
      input,
      requiresApproval: askFirst,
    })
    const failure = await saveWorkflowYaml(workflow.id, yaml)
    setSaving(false)
    if (failure) {
      setError(failure)
      return
    }
    setSaved(true)
    onStepRenamed?.(step.id)
  }

  return (
    <section className="flex flex-col gap-3 rounded-md border p-3">
      <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {t('workflowBuilder.editStep')}
      </h3>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="step-edit-agent">
          {t('workflowBuilder.fields.agent')}
        </Label>
        <NativeSelect
          id="step-edit-agent"
          value={agent}
          onChange={e => setAgent(e.target.value)}
        >
          {!known && (
            <NativeSelectOption value={agent}>
              {t('automations.step.missingOption', { id: agent })}
            </NativeSelectOption>
          )}
          {agents.map(a => (
            <NativeSelectOption key={a.id} value={a.id}>
              {a.name}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="step-edit-input">{t('automations.step.message')}</Label>
        <Textarea
          id="step-edit-input"
          dir="auto"
          className="text-sm"
          value={input}
          onChange={e => setInput(e.target.value)}
        />
      </div>
      <label className="flex items-center gap-2 text-sm">
        <Checkbox
          checked={askFirst}
          onCheckedChange={checked => setAskFirst(checked === true)}
        />
        {t('automations.step.askFirst')}
      </label>
      {active && changed && (
        <p className="text-xs text-muted-foreground">
          {t('automations.step.nextRun')}
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        {saved && !changed && (
          <span className="flex items-center gap-1 text-xs text-success">
            <Check aria-hidden className="size-3.5" />
            {t('automations.step.saved')}
          </span>
        )}
        <Button
          size="sm"
          disabled={!changed || saving || !agent}
          onClick={() => void save()}
        >
          {t('workflowBuilder.saveStep')}
        </Button>
      </div>
    </section>
  )
}
