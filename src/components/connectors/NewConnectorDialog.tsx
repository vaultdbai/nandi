import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Plus, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { useScaffoldConnector } from '@/hooks/useConnectorHub'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import {
  checkScaffold,
  EMPTY_SCAFFOLD,
  scaffoldInput,
  type ScaffoldError,
  type ScaffoldField,
  type ScaffoldForm,
  type ToolDraft,
} from './scaffold-model'
import { PublishConnectorDialog } from './PublishConnectorDialog'

/**
 * Studio › New › Connector: a connector.md from a form. Create saves it
 * (Nandi starts the server once to list its tools when none are listed);
 * then it can be published to one of the person's registries.
 */
export function NewConnectorDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const [created, setCreated] = useState<ConnectorDetails | null>(null)
  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
          {open && (
            <ConnectorFormBody
              onCancel={() => onOpenChange(false)}
              onCreated={connector => {
                onOpenChange(false)
                setCreated(connector)
              }}
            />
          )}
        </DialogContent>
      </Dialog>
      <CreatedDialog connector={created} onClose={() => setCreated(null)} />
    </>
  )
}

function CreatedDialog({
  connector,
  onClose,
}: {
  connector: ConnectorDetails | null
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [publishing, setPublishing] = useState(false)
  return (
    <>
      <Dialog
        open={connector !== null && !publishing}
        onOpenChange={next => !next && onClose()}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {t('newConnector.createdTitle', { name: connector?.name ?? '' })}
            </DialogTitle>
            <DialogDescription>
              {t('newConnector.createdBody')}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={onClose}>
              {t('publishConnector.done')}
            </Button>
            <Button onClick={() => setPublishing(true)}>
              {t('packageConnectors.publish')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <PublishConnectorDialog
        connector={publishing ? connector : null}
        onOpenChange={next => {
          if (!next) {
            setPublishing(false)
            onClose()
          }
        }}
      />
    </>
  )
}

function ConnectorFormBody({
  onCancel,
  onCreated,
}: {
  onCancel: () => void
  onCreated: (connector: ConnectorDetails) => void
}) {
  const { t } = useTranslation()
  const [form, setForm] = useState<ScaffoldForm>(EMPTY_SCAFFOLD)
  const [tried, setTried] = useState(false)
  const scaffold = useScaffoldConnector()
  const errors = tried ? checkScaffold(form) : []
  const set = <K extends keyof ScaffoldForm>(key: K, value: ScaffoldForm[K]) =>
    setForm(current => ({ ...current, [key]: value }))
  const errorFor = (field: ScaffoldField) =>
    errors.filter(error => error.field === field)

  const submit = () => {
    setTried(true)
    if (checkScaffold(form).length > 0) return
    scaffold.mutate(scaffoldInput(form), {
      onSuccess: connector => {
        toast.success(t('newConnector.created', { name: connector.name }))
        onCreated(connector as ConnectorDetails)
      },
    })
  }

  const setTool = (index: number, tool: ToolDraft) =>
    set(
      'tools',
      form.tools.map((current, i) => (i === index ? tool : current))
    )

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t('newConnector.title')}</DialogTitle>
        <DialogDescription>{t('newConnector.description')}</DialogDescription>
      </DialogHeader>
      <div className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <TextInput
            label={t('newConnector.id')}
            value={form.id}
            onChange={value => set('id', value)}
            errors={errorFor('id')}
            ltr
          />
          <TextInput
            label={t('newConnector.name')}
            value={form.name}
            onChange={value => set('name', value)}
            errors={errorFor('name')}
          />
        </div>
        <TextInput
          label={t('newConnector.descriptionLabel')}
          value={form.description}
          onChange={value => set('description', value)}
          errors={errorFor('description')}
        />
        <TextInput
          label={t('newConnector.version')}
          value={form.version}
          onChange={value => set('version', value)}
          errors={errorFor('version')}
          ltr
        />

        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">
            {t('newConnector.server')}
          </legend>
          <NativeSelect
            aria-label={t('newConnector.server')}
            value={form.server}
            onChange={event =>
              set('server', event.target.value === 'url' ? 'url' : 'package')
            }
          >
            <NativeSelectOption value="package">
              {t('newConnector.serverPackage')}
            </NativeSelectOption>
            <NativeSelectOption value="url">
              {t('newConnector.serverUrl')}
            </NativeSelectOption>
          </NativeSelect>
          {form.server === 'package' ? (
            <TextInput
              label={t('newConnector.run')}
              hint={t('newConnector.runHint')}
              placeholder="npx -y @scope/pkg@1.2.3"
              value={form.run}
              onChange={value => set('run', value)}
              errors={errorFor('run')}
              ltr
            />
          ) : (
            <TextInput
              label={t('newConnector.url')}
              placeholder="https://mcp.example.com/mcp"
              value={form.url}
              onChange={value => set('url', value)}
              errors={errorFor('url')}
              ltr
            />
          )}
        </fieldset>

        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">
            {t('newConnector.auth')}
          </legend>
          <NativeSelect
            aria-label={t('newConnector.auth')}
            value={form.auth}
            onChange={event =>
              set(
                'auth',
                event.target.value === 'api_key'
                  ? 'api_key'
                  : event.target.value === 'oauth'
                    ? 'oauth'
                    : 'none'
              )
            }
          >
            <NativeSelectOption value="none">
              {t('newConnector.authNone')}
            </NativeSelectOption>
            <NativeSelectOption value="api_key">
              {t('newConnector.authApiKey')}
            </NativeSelectOption>
            <NativeSelectOption value="oauth">
              {t('newConnector.authOauth')}
            </NativeSelectOption>
          </NativeSelect>
          {form.auth === 'api_key' && (
            <div className="grid gap-4 sm:grid-cols-2">
              <TextInput
                label={t('newConnector.authEnv')}
                hint={t('newConnector.authEnvHint')}
                placeholder="TRACKER_TOKEN"
                value={form.authEnv}
                onChange={value => set('authEnv', value)}
                errors={errorFor('authEnv')}
                ltr
              />
              <TextInput
                label={t('newConnector.authLabel')}
                value={form.authLabel}
                onChange={value => set('authLabel', value)}
              />
              <TextInput
                label={t('newConnector.apiKey')}
                hint={t('newConnector.apiKeyHint')}
                value={form.apiKey}
                onChange={value => set('apiKey', value)}
                password
              />
            </div>
          )}
          {form.auth === 'oauth' && (
            <TextInput
              label={t('newConnector.authScopes')}
              value={form.authScopes}
              onChange={value => set('authScopes', value)}
              ltr
            />
          )}
        </fieldset>

        <TextArea
          label={t('newConnector.hosts')}
          hint={t('newConnector.hostsHint')}
          value={form.hosts}
          onChange={value => set('hosts', value)}
          errors={errorFor('hosts')}
          ltr
        />

        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">
            {t('newConnector.tools')}
          </legend>
          {form.tools.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {t('newConnector.toolsEmpty')}
            </p>
          )}
          {form.tools.map((tool, index) => (
            <div
              // Tools have no id until named; the row's place is its key
              key={index}
              className="flex flex-wrap items-center gap-2"
            >
              <Input
                aria-label={t('newConnector.toolName')}
                placeholder={t('newConnector.toolName')}
                dir="ltr"
                className="w-40"
                value={tool.name}
                onChange={event =>
                  setTool(index, { ...tool, name: event.target.value })
                }
              />
              <Input
                aria-label={t('newConnector.toolDescription')}
                placeholder={t('newConnector.toolDescription')}
                className="min-w-40 flex-1"
                value={tool.description}
                onChange={event =>
                  setTool(index, { ...tool, description: event.target.value })
                }
              />
              <NativeSelect
                aria-label={t('newConnector.toolAccess')}
                value={tool.access}
                onChange={event =>
                  setTool(index, {
                    ...tool,
                    access: event.target.value === 'read' ? 'read' : 'write',
                  })
                }
              >
                <NativeSelectOption value="read">
                  {t('grants.access.read')}
                </NativeSelectOption>
                <NativeSelectOption value="write">
                  {t('grants.access.write')}
                </NativeSelectOption>
              </NativeSelect>
              <Button
                size="icon"
                variant="ghost"
                aria-label={t('newConnector.removeTool')}
                onClick={() =>
                  set(
                    'tools',
                    form.tools.filter((_, i) => i !== index)
                  )
                }
              >
                <Trash2 />
              </Button>
            </div>
          ))}
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              set('tools', [
                ...form.tools,
                { name: '', description: '', access: 'read' },
              ])
            }
          >
            <Plus />
            {t('newConnector.addTool')}
          </Button>
          <FieldErrors errors={errorFor('tools')} />
        </fieldset>

        <div className="grid gap-4 sm:grid-cols-2">
          <TextArea
            label={t('newConnector.reads')}
            hint={t('newConnector.linesHint')}
            value={form.reads}
            onChange={value => set('reads', value)}
          />
          <TextArea
            label={t('newConnector.writes')}
            hint={t('newConnector.linesHint')}
            value={form.writes}
            onChange={value => set('writes', value)}
          />
        </div>
        <TextInput
          label={t('newConnector.tags')}
          hint={t('newConnector.tagsHint')}
          value={form.tags}
          onChange={value => set('tags', value)}
        />
        <TextArea
          label={t('newConnector.instructions')}
          hint={t('newConnector.instructionsHint')}
          value={form.instructions}
          onChange={value => set('instructions', value)}
          errors={errorFor('instructions')}
          rows={5}
        />

        {form.server === 'package' && (
          <Collapsible>
            <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-2 hover:underline">
              {t('newConnector.advanced')}
            </CollapsibleTrigger>
            <CollapsibleContent className="pt-2">
              <TextInput
                label={t('newConnector.image')}
                hint={t('newConnector.imageHint')}
                value={form.image}
                onChange={value => set('image', value)}
                errors={errorFor('image')}
                ltr
              />
            </CollapsibleContent>
          </Collapsible>
        )}
        {scaffold.error && (
          <p className="text-sm text-destructive">{String(scaffold.error)}</p>
        )}
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={onCancel}>
          {t('connectorConsent.cancel')}
        </Button>
        <Button disabled={scaffold.isPending} onClick={submit}>
          {scaffold.isPending && <Spinner />}
          {scaffold.isPending && form.tools.length === 0
            ? t('newConnector.listingTools')
            : t('newConnector.create')}
        </Button>
      </DialogFooter>
    </>
  )
}

function FieldErrors({ errors }: { errors: ScaffoldError[] }) {
  const { t } = useTranslation()
  if (errors.length === 0) return null
  return (
    <>
      {errors.map(error => (
        <p
          key={`${error.key}-${JSON.stringify(error.values ?? {})}`}
          className="text-xs text-destructive"
        >
          {t(`newConnector.error.${error.key}`, error.values)}
        </p>
      ))}
    </>
  )
}

interface InputProps {
  label: string
  value: string
  onChange: (value: string) => void
  hint?: string
  placeholder?: string
  errors?: ScaffoldError[]
  ltr?: boolean
}

function TextInput({
  label,
  value,
  onChange,
  hint,
  placeholder,
  errors = [],
  ltr,
  password,
}: InputProps & { password?: boolean }) {
  const id = useId()
  return (
    <div className="space-y-1">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type={password ? 'password' : 'text'}
        autoComplete={password ? 'off' : undefined}
        dir={ltr ? 'ltr' : undefined}
        placeholder={placeholder}
        value={value}
        aria-invalid={errors.length > 0 || undefined}
        onChange={event => onChange(event.target.value)}
      />
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      <FieldErrors errors={errors} />
    </div>
  )
}

function TextArea({
  label,
  value,
  onChange,
  hint,
  errors = [],
  ltr,
  rows = 3,
}: InputProps & { rows?: number }) {
  const id = useId()
  return (
    <div className="space-y-1">
      <Label htmlFor={id}>{label}</Label>
      <Textarea
        id={id}
        rows={rows}
        dir={ltr ? 'ltr' : undefined}
        value={value}
        aria-invalid={errors.length > 0 || undefined}
        onChange={event => onChange(event.target.value)}
      />
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      <FieldErrors errors={errors} />
    </div>
  )
}
