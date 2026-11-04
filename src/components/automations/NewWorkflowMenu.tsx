import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import {
  ChevronDown,
  FileUp,
  LayoutTemplate,
  Plus,
  Sparkles,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Spinner } from '@/components/ui/spinner'
import { useWorkflowImport } from '@/components/views/WorkflowImport'
import { useAgents } from '@/hooks/useAgents'
import { sendToStudio } from '@/lib/agent-actions'
import type { WorkflowTemplate } from '@/lib/bindings'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { createWorkflowFromTemplate } from '@/lib/workflow-actions'
import { useNavigationStore } from '@/store/navigation-store'
import { stepLabel } from './automations-model'

/** Opens Studio with a request to draft a workflow */
function describeInStudio(text: string) {
  void sendToStudio(text, [], null)
}

/**
 * "New workflow ▾": From a template (the dialog) · Describe it in Studio ·
 * Import YAML file…. The file picker no longer sits behind "New".
 */
export function NewWorkflowMenu({
  variant = 'outline',
}: {
  variant?: 'default' | 'outline'
}) {
  const { t } = useTranslation()
  const [templates, setTemplates] = useState(false)
  const { pick, input } = useWorkflowImport()

  return (
    <>
      {input}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm" variant={variant}>
            <Plus />
            {t('automations.newWorkflow')}
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => setTemplates(true)}>
            <LayoutTemplate />
            {t('automations.fromTemplate')}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => describeInStudio(t('automations.studioPrompt'))}
          >
            <Sparkles />
            {t('automations.describeInStudio')}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={pick}>
            <FileUp />
            {t('automations.importYaml')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <TemplateDialog open={templates} onOpenChange={setTemplates} />
    </>
  )
}

/** The empty Workflows list: a template first, then Studio or a file */
export function WorkflowsEmptyActions() {
  const { t } = useTranslation()
  const [templates, setTemplates] = useState(false)
  const { pick, input } = useWorkflowImport()

  return (
    <div className="flex flex-col items-center gap-2">
      {input}
      <div className="flex flex-wrap justify-center gap-2">
        <Button onClick={() => setTemplates(true)}>
          <LayoutTemplate />
          {t('automations.startFromTemplate')}
        </Button>
        <Button
          variant="outline"
          onClick={() => describeInStudio(t('automations.studioPrompt'))}
        >
          <Sparkles />
          {t('automations.describeInStudio')}
        </Button>
      </div>
      <Button variant="link" size="sm" onClick={pick}>
        {t('automations.orImportYaml')}
      </Button>
      <TemplateDialog open={templates} onOpenChange={setTemplates} />
    </div>
  )
}

export function TemplateDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const { data: templates } = useQuery({
    queryKey: ['workflowTemplates'],
    queryFn: async () => unwrapResult(await client.listWorkflowTemplates()),
    enabled: open,
    staleTime: Infinity,
  })

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t('automations.templates.title')}</DialogTitle>
          <DialogDescription>
            {t('automations.templates.description')}
          </DialogDescription>
        </DialogHeader>
        {templates ? (
          <TemplateForm
            templates={templates}
            onDone={() => onOpenChange(false)}
          />
        ) : (
          <div className="flex justify-center py-6">
            <Spinner />
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function TemplateForm({
  templates,
  onDone,
}: {
  templates: WorkflowTemplate[]
  onDone: () => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const { data: agents = [] } = useAgents()
  const [templateId, setTemplateId] = useState(templates[0]?.id ?? '')
  const [name, setName] = useState<string | null>(null)
  const [chosen, setChosen] = useState<Record<string, string>>({})
  const [creating, setCreating] = useState(false)
  const template = templates.find(tpl => tpl.id === templateId)
  const defaultName = template
    ? t(`automations.templates.${template.id}.name`, {
        defaultValue: template.name,
      })
    : ''
  const agentSteps = template?.steps.filter(step => step.needsAgent) ?? []
  const missing = agentSteps.some(step => !chosen[step.id])

  const create = async () => {
    if (!template) return
    setCreating(true)
    const created = await createWorkflowFromTemplate({
      templateId: template.id,
      name: (name ?? defaultName).trim(),
      agents: Object.fromEntries(
        agentSteps.map(step => [step.id, chosen[step.id] ?? ''])
      ),
    })
    setCreating(false)
    if (created) onDone()
  }

  return (
    <div className="flex flex-col gap-4">
      <RadioGroup
        value={templateId}
        onValueChange={value => {
          setTemplateId(value)
          setName(null)
        }}
        aria-label={t('automations.templates.title')}
      >
        {templates.map(tpl => (
          <label
            key={tpl.id}
            className="flex items-start gap-3 rounded-md border p-3 text-sm"
          >
            <RadioGroupItem value={tpl.id} className="mt-0.5" />
            <span className="flex flex-col gap-0.5">
              <span className="font-medium">
                {t(`automations.templates.${tpl.id}.name`, {
                  defaultValue: tpl.name,
                })}
              </span>
              <span className="text-xs text-muted-foreground">
                {t(`automations.templates.${tpl.id}.summary`)}
              </span>
            </span>
          </label>
        ))}
      </RadioGroup>

      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${id}-name`}>{t('automations.templates.name')}</Label>
        <Input
          id={`${id}-name`}
          value={name ?? defaultName}
          onChange={event => setName(event.target.value)}
        />
      </div>

      {agentSteps.length > 0 && agents.length === 0 ? (
        <div className="flex flex-col items-start gap-2 rounded-md border border-dashed p-3 text-sm">
          <p className="text-muted-foreground">
            {t('automations.templates.needsAgent')}
          </p>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              onDone()
              useNavigationStore.getState().navigate({ kind: 'starter-packs' })
            }}
          >
            {t('automations.templates.installPack')}
          </Button>
        </div>
      ) : (
        agentSteps.map(step => (
          <div key={step.id} className="flex flex-col gap-1.5">
            <Label htmlFor={`${id}-${step.id}`}>
              {t('automations.templates.stepUses', {
                step: stepLabel(step.id),
              })}
            </Label>
            <NativeSelect
              id={`${id}-${step.id}`}
              value={chosen[step.id] ?? ''}
              onChange={event =>
                setChosen(current => ({
                  ...current,
                  [step.id]: event.target.value,
                }))
              }
            >
              <NativeSelectOption value="">
                {t('automations.chooseAgent')}
              </NativeSelectOption>
              {agents.map(agent => (
                <NativeSelectOption key={agent.id} value={agent.id}>
                  {agent.name}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
        ))
      )}

      <DialogFooter>
        <Button variant="outline" onClick={onDone}>
          {t('automations.cancel')}
        </Button>
        <Button
          disabled={!template || missing || creating}
          onClick={() => void create()}
        >
          {creating && <Spinner />}
          {t('automations.templates.create')}
        </Button>
      </DialogFooter>
    </div>
  )
}
