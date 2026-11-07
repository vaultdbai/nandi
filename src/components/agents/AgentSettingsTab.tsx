import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ChevronDown, Info, Lock, PencilRuler } from 'lucide-react'
import { AgentMemoryPanel } from '@/components/agent-console/AgentMemoryPanel'
import { ConnectorsPanel } from '@/components/agent-console/ConnectorsPanel'
import { DelegationPanel } from '@/components/agent-console/DelegationPanel'
import { PermissionsPanel } from '@/components/agent-console/PermissionsPanel'
import { AgentPolicies } from './AgentPolicies'
import { AwsProfileField } from './AwsProfileField'
import { AgentDelegationTab } from '@/components/agents/AgentDelegationTab'
import { runtimeOptions } from '@/components/harness/runtime-options'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import {
  toDefinitionInput,
  useAgentDefinition,
  useResetAgentInstructions,
  useSaveAgent,
  useSaveAgentInstructions,
  validateDraft,
  type EditorDraft,
} from '@/hooks/useAgentEditor'
import {
  useAgentConnectors,
  useSetAgentConnectors,
} from '@/hooks/useConnectors'
import { useHarnesses } from '@/hooks/useHarnesses'
import {
  useDefaultModel,
  useLlmKeyStatus,
  useOllama,
} from '@/hooks/useLlmKeyStatus'
import type { AgentConnectorChoice, HostRule } from '@/lib/bindings'
import { openAgentInStudio } from '@/lib/ask'
import { modelLabel } from '@/lib/models'
import { openSettings } from '@/lib/settings-nav'
import { cn } from '@/lib/utils'
import type { AgentInfo } from '@/types/agent'
import { DeleteAgentDialog } from './DeleteAgentDialog'
import { NetworkAccess } from './NetworkAccess'
import {
  hasModelFormatError,
  isCliRuntime,
  modelOptions,
  providerChoices,
  resourceError,
  runtimeName,
} from './agent-page-model'
import { AgentConnectorGrants } from '@/components/connectors/AgentConnectorGrants'

const sameDraft = (a: EditorDraft, b: EditorDraft) =>
  JSON.stringify(a) === JSON.stringify(b)

/**
 * How the agent is set up, in plain words: name, model (provider and
 * model by name, or a CLI runtime and where it runs), what it may do,
 * sharing, and container resources under Advanced. Instructions, tools
 * and new abilities are changed in Studio.
 */
export function AgentSettingsTab({
  agent,
  onDirtyChange,
}: {
  agent: AgentInfo
  onDirtyChange: (dirty: boolean) => void
}) {
  const { t } = useTranslation()
  const definition = useAgentDefinition(agent.id)

  if (definition.isPending) {
    return (
      <div className="mx-auto w-full max-w-3xl space-y-3 p-4">
        <Skeleton className="h-24" />
        <Skeleton className="h-32" />
      </div>
    )
  }
  if (definition.isError) {
    const badModel = hasModelFormatError(definition.error.message)
    return (
      <div className="mx-auto w-full max-w-3xl p-4">
        <Alert variant="destructive">
          <AlertDescription className="flex flex-wrap items-center gap-2">
            {badModel
              ? t('agentPage.problem.badModel')
              : t('agentPage.settings.loadError')}
            {agent.origin.kind === 'local' && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => openAgentInStudio(agent.id)}
              >
                {t('agentPage.editInStudio')}
              </Button>
            )}
          </AlertDescription>
        </Alert>
      </div>
    )
  }

  return (
    <SettingsForm
      // A fresh form whenever the files change (e.g. Studio saved, or
      // Your rules saved)
      key={`${definition.data.renderedSkillMd}\n${definition.data.draft.instructions}`}
      agent={agent}
      saved={definition.data.draft}
      networkRules={definition.data.networkRules}
      readOnly={definition.data.readOnly}
      instructionsOverridden={definition.data.instructionsOverridden}
      usesAws={
        definition.data.hasAws || definition.data.draft.awsProfile !== ''
      }
      onDirtyChange={onDirtyChange}
    />
  )
}

function SettingsForm({
  agent,
  saved,
  networkRules,
  readOnly,
  instructionsOverridden,
  usesAws,
  onDirtyChange,
}: {
  agent: AgentInfo
  saved: EditorDraft
  networkRules: HostRule[]
  readOnly: boolean
  instructionsOverridden: boolean
  /** Shows the AWS profile: only agents that call AWS need one */
  usesAws: boolean
  onDirtyChange: (dirty: boolean) => void
}) {
  const { t } = useTranslation()
  const save = useSaveAgent(agent.id)
  const [draft, setDraft] = useState(saved)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [revision, setRevision] = useState(0)
  const dirty = !sameDraft(draft, saved)
  const errors = validateDraft(draft)
  const memoryError = resourceError('memoryMb', draft.memoryMb)
  const cpuError = resourceError('cpu', draft.cpu)
  const invalid =
    Object.keys(errors).length > 0 || memoryError !== null || cpuError !== null

  useEffect(() => {
    onDirtyChange(dirty)
  }, [dirty, onDirtyChange])

  const update = (patch: Partial<EditorDraft>) => {
    setSaveError(null)
    setDraft(current => ({ ...current, ...patch }))
  }

  const submit = () =>
    save.mutate(toDefinitionInput(draft), {
      onSuccess: () => {
        toast.success(t('agentPage.settings.saved'))
        onDirtyChange(false)
      },
      onError: error => {
        const message = error instanceof Error ? error.message : String(error)
        // Never the raw format: say what to do
        setSaveError(
          hasModelFormatError(message)
            ? t('agentPage.settings.pickModel')
            : message
        )
      },
    })

  return (
    <div className="flex min-h-full flex-col">
      <div className="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-6 p-4">
        {readOnly && (
          <Alert>
            <Lock />
            <AlertDescription>
              {t('agentPage.settings.readOnly')}
            </AlertDescription>
          </Alert>
        )}
        <fieldset disabled={readOnly} className="flex flex-col gap-6">
          <Section title={t('agentPage.settings.general')}>
            <Field
              id="agent-name"
              label={t('agentPage.settings.name')}
              error={errors.name && t(errors.name, { max: 200 })}
            >
              <Input
                id="agent-name"
                value={draft.name}
                onChange={event => update({ name: event.target.value })}
              />
            </Field>
            <Field
              id="agent-description"
              label={t('agentPage.settings.description')}
            >
              <Textarea
                id="agent-description"
                rows={2}
                value={draft.description}
                onChange={event => update({ description: event.target.value })}
              />
            </Field>
          </Section>

          <ModelSection draft={draft} update={update} />

          <Section title={t('agentPage.network.title')}>
            <NetworkAccess
              // Fresh after Revert, which resets the sites under it
              key={revision}
              sites={draft.network}
              rules={networkRules}
              onChange={network => update({ network })}
            />
          </Section>

          {usesAws && (
            <Section title={t('agentPage.settings.aws')}>
              <AwsProfileField
                value={draft.awsProfile}
                onChange={awsProfile => update({ awsProfile })}
              />
            </Section>
          )}
        </fieldset>

        <YourRules
          agentId={agent.id}
          saved={saved.instructions}
          installed={readOnly}
          overridden={instructionsOverridden}
        />

        <Section
          title={t('agentPage.settings.permissions')}
          action={
            agent.origin.kind === 'local' && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => openAgentInStudio(agent.id)}
              >
                <PencilRuler />
                {t('agentPage.settings.changeInStudio')}
              </Button>
            )
          }
        >
          <AgentPolicies agentId={agent.id} />
          <div className="rounded-md border">
            <PermissionsPanel agentId={agent.id} />
          </div>
          <AgentConnectors agentId={agent.id} />
          <AgentConnectorGrants agentId={agent.id} />
        </Section>

        <Section title={t('agentPage.settings.sharing')}>
          <div className="rounded-md border">
            <DelegationPanel agentId={agent.id} agentName={agent.name} />
          </div>
          <AgentDelegationTab agentId={agent.id} />
        </Section>

        <Collapsible>
          <CollapsibleTrigger className="group flex items-center gap-1 text-sm font-medium">
            <ChevronDown className="size-4 transition-transform group-data-[state=closed]:-rotate-90 rtl:group-data-[state=closed]:rotate-90" />
            {t('agentPage.settings.advanced')}
          </CollapsibleTrigger>
          <CollapsibleContent className="mt-3 flex flex-col gap-4">
            <fieldset disabled={readOnly} className="grid gap-4 sm:grid-cols-3">
              <Field
                id="agent-image"
                label={t('agentPage.settings.image')}
                error={errors.image && t(errors.image)}
              >
                <Input
                  id="agent-image"
                  dir="ltr"
                  value={draft.image}
                  placeholder={t('agentPage.settings.imageDefault')}
                  onChange={event => update({ image: event.target.value })}
                />
              </Field>
              <Field
                id="agent-memory"
                label={t('agentPage.settings.memory')}
                error={memoryError && t('agentPage.settings.memoryRange')}
              >
                <Input
                  id="agent-memory"
                  inputMode="numeric"
                  value={draft.memoryMb}
                  placeholder="1024"
                  onChange={event => update({ memoryMb: event.target.value })}
                />
              </Field>
              <Field
                id="agent-cpu"
                label={t('agentPage.settings.cpu')}
                error={cpuError && t('agentPage.settings.cpuRange')}
              >
                <Input
                  id="agent-cpu"
                  inputMode="decimal"
                  value={draft.cpu}
                  placeholder="1"
                  onChange={event => update({ cpu: event.target.value })}
                />
              </Field>
            </fieldset>
            <AgentMemoryPanel agentId={agent.id} />
          </CollapsibleContent>
        </Collapsible>

        {agent.origin.kind === 'local' && !agent.builtin && (
          <Section title={t('agentPage.settings.danger')}>
            <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-destructive/30 p-3 text-sm">
              <p>{t('agentPage.settings.deleteHint')}</p>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => setDeleting(true)}
              >
                {t('agentPage.delete.action')}
              </Button>
            </div>
            <DeleteAgentDialog
              agent={agent}
              open={deleting}
              onOpenChange={setDeleting}
            />
          </Section>
        )}
      </div>

      {/* End padding keeps Save clear of the docked companion (bottom end) */}
      {dirty && !readOnly && (
        <div className="sticky bottom-0 flex flex-wrap items-center justify-end gap-2 border-t bg-background py-3 ps-4 pe-52">
          {saveError && (
            <p role="alert" className="me-auto text-sm text-destructive">
              {saveError}
            </p>
          )}
          <Button
            variant="ghost"
            onClick={() => {
              update(saved)
              setRevision(current => current + 1)
            }}
          >
            {t('agentPage.settings.revert')}
          </Button>
          <Button disabled={invalid || save.isPending} onClick={submit}>
            {save.isPending && <Spinner />}
            {t('agentPage.settings.save')}
          </Button>
        </div>
      )}
    </div>
  )
}

/** Runtime, then the model by name (Nandi) or where it runs (a CLI) */
function ModelSection({
  draft,
  update,
}: {
  draft: EditorDraft
  update: (patch: Partial<EditorDraft>) => void
}) {
  const { t } = useTranslation()
  const harnesses = useHarnesses()
  const keys = useLlmKeyStatus()
  const ollama = useOllama()
  const { data: defaultModel } = useDefaultModel()
  const runtimes = runtimeOptions(
    (harnesses.data ?? []).map(h => ({
      id: h.id,
      displayName: h.displayName,
      installed: h.health.installed || h.imageAvailable,
    })),
    draft.runtime
  )
  const choices = providerChoices(
    keys.data,
    ollama.data?.running ? ollama.data.models : null
  )
  const provider = choices.find(c => c.prefix === draft.modelProvider)
  const models = modelOptions(choices, draft.modelProvider, draft.modelName)
  const cli = isCliRuntime(draft.runtime)
  const harness = harnesses.data?.find(h => h.id === draft.runtime)
  const defaultName = defaultModel?.defaultModel
    ? modelLabel(defaultModel.defaultModel)
    : ''

  return (
    <Section title={t('agentPage.settings.model')}>
      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">
          {t('agentPage.settings.runtime')}
        </legend>
        <RadioGroup
          value={draft.runtime}
          onValueChange={runtime => update({ runtime })}
          className="flex flex-wrap gap-3"
        >
          {runtimes.map(option => (
            <div key={option.id} className="flex items-center gap-2">
              <RadioGroupItem
                id={`runtime-${option.id}`}
                value={option.id}
                disabled={!option.installed && option.id !== draft.runtime}
              />
              <Label
                htmlFor={`runtime-${option.id}`}
                className={cn(!option.installed && 'text-muted-foreground')}
              >
                {runtimeName(option.id)}
                {!option.installed &&
                  ` · ${t('agentPage.settings.notInstalled')}`}
              </Label>
            </div>
          ))}
        </RadioGroup>
      </fieldset>

      {cli ? (
        <>
          {/* Agents run only in the container; there's nothing to choose */}
          <div className="space-y-1">
            <p className="text-sm font-medium">
              {t('agentPage.settings.whereItRuns')}
            </p>
            <p className="text-sm text-muted-foreground">
              {t('agentPage.settings.placement.container')}
            </p>
          </div>
          <Field
            id="agent-runtime-model"
            label={t('agentPage.settings.cliModel')}
          >
            <Input
              id="agent-runtime-model"
              dir="ltr"
              value={draft.runtimeModel}
              placeholder={t('agentPage.settings.cliModelDefault', {
                runtime: runtimeName(draft.runtime),
              })}
              onChange={event => update({ runtimeModel: event.target.value })}
            />
          </Field>
          {harness && !harness.health.containerAuthenticated && (
            <Note
              action={
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => openSettings('models')}
                >
                  {t('agentPage.health.addKey')}
                </Button>
              }
            >
              {t('agentPage.settings.hostLoginNote', {
                runtime: runtimeName(draft.runtime),
              })}
            </Note>
          )}
        </>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          <Field id="agent-provider" label={t('agentPage.settings.provider')}>
            <NativeSelect
              id="agent-provider"
              value={draft.modelProvider}
              onChange={event => {
                const prefix = event.target.value
                const first = choices.find(c => c.prefix === prefix)?.models[0]
                update({
                  modelProvider: prefix,
                  modelName: prefix ? (first?.id ?? '') : '',
                })
              }}
            >
              <NativeSelectOption value="">
                {defaultName
                  ? t('agentPage.model.defaultNamed', { model: defaultName })
                  : t('agentPage.model.default')}
              </NativeSelectOption>
              {choices.map(choice => (
                <NativeSelectOption
                  key={choice.prefix}
                  value={choice.prefix}
                  disabled={
                    !choice.usable && choice.prefix !== draft.modelProvider
                  }
                >
                  {t(choice.labelKey)}
                  {!choice.usable && ` · ${t('agentPage.settings.needsKey')}`}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
          {draft.modelProvider && (
            <Field id="agent-model" label={t('agentPage.settings.modelName')}>
              <NativeSelect
                id="agent-model"
                value={draft.modelName}
                onChange={event => update({ modelName: event.target.value })}
              >
                {models.map(model => (
                  <NativeSelectOption
                    key={model.id}
                    value={model.id}
                    title={`${draft.modelProvider}:${model.id}`}
                  >
                    {model.label}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </Field>
          )}
          {provider && !provider.usable && (
            <Note
              action={
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => openSettings('models')}
                >
                  {t('agentPage.settings.addKey')}
                </Button>
              }
            >
              {t('agentPage.settings.providerNoKey', {
                provider: t(provider.labelKey),
              })}
            </Note>
          )}
          {!draft.modelProvider && (
            <p className="text-xs text-muted-foreground sm:col-span-2">
              {t('agentPage.settings.defaultHint')}
            </p>
          )}
        </div>
      )}
    </Section>
  )
}

/**
 * The agent's instructions.md: the user's rules. Editable for every
 * agent, installed ones included: theirs are kept as an override that
 * updates don't touch, and Reset goes back to the shipped rules.
 */
function YourRules({
  agentId,
  saved,
  installed,
  overridden,
}: {
  agentId: string
  saved: string
  installed: boolean
  overridden: boolean
}) {
  const { t } = useTranslation()
  const save = useSaveAgentInstructions(agentId)
  const reset = useResetAgentInstructions(agentId)
  const [text, setText] = useState(saved)
  const dirty = text.trim() !== saved.trim()
  const busy = save.isPending || reset.isPending
  const fail = (error: unknown) =>
    toast.error(error instanceof Error ? error.message : String(error))

  return (
    <Section
      title={t('agentPage.rules.title')}
      action={
        installed &&
        overridden && (
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() =>
              reset.mutate(undefined, {
                onSuccess: () => toast.success(t('agentPage.rules.resetDone')),
                onError: fail,
              })
            }
          >
            {t('agentPage.rules.reset')}
          </Button>
        )
      }
    >
      <p className="text-xs text-muted-foreground">
        {installed
          ? overridden
            ? t('agentPage.rules.hintOverridden')
            : t('agentPage.rules.hintInstalled')
          : t('agentPage.rules.hintLocal')}
      </p>
      <Textarea
        aria-label={t('agentPage.rules.title')}
        rows={10}
        className="font-mono text-xs"
        value={text}
        onChange={event => setText(event.target.value)}
      />
      {dirty && (
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => setText(saved)}>
            {t('agentPage.settings.revert')}
          </Button>
          <Button
            size="sm"
            disabled={busy}
            onClick={() =>
              save.mutate(text, {
                onSuccess: () => toast.success(t('agentPage.rules.saved')),
                onError: fail,
              })
            }
          >
            {save.isPending && <Spinner />}
            {t('agentPage.rules.save')}
          </Button>
        </div>
      )}
    </Section>
  )
}

function Section({
  title,
  action,
  children,
}: {
  title: string
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2 border-b pb-1">
        <h2 className="text-sm font-semibold">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  )
}

function Field({
  id,
  label,
  error,
  children,
}: {
  id: string
  label: string
  error?: string | null | false
  children: React.ReactNode
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}

function Note({
  children,
  action,
}: {
  children: React.ReactNode
  action?: React.ReactNode
}) {
  return (
    <div className="flex flex-wrap items-start gap-2 rounded-md border bg-muted/40 p-2 text-xs sm:col-span-2">
      <Info
        aria-hidden
        className="mt-0.5 size-4 shrink-0 text-muted-foreground"
      />
      <p className="min-w-0 flex-1">{children}</p>
      {action}
    </div>
  )
}

/**
 * The connectors and shared folders this agent uses. Each change is saved
 * to its skill.md right away (like the permission switches above).
 */
function AgentConnectors({ agentId }: { agentId: string }) {
  const { t } = useTranslation()
  const { data: choices } = useAgentConnectors(agentId)
  const save = useSetAgentConnectors(agentId)
  const current = choices ?? []
  const filesystem = current.find(c => c.id === 'filesystem')

  const commit = (next: AgentConnectorChoice[]) =>
    save.mutate(next, {
      onSuccess: () => toast.success(t('agentPage.settings.saved')),
      onError: error => toast.error(error.message),
    })

  return (
    <ConnectorsPanel
      agentId={agentId}
      enabled={current.map(c => c.id)}
      onEnabledChange={
        choices
          ? enabled =>
              commit(
                enabled.map(id => current.find(c => c.id === id) ?? { id })
              )
          : undefined
      }
      folders={filesystem?.folders ?? []}
      onFoldersChange={
        choices
          ? folders =>
              commit(
                current.map(c =>
                  c.id === 'filesystem'
                    ? {
                        ...c,
                        folders: folders.map(f => ({
                          id: f.id,
                          access: f.access ?? null,
                        })),
                      }
                    : c
                )
              )
          : undefined
      }
    />
  )
}
