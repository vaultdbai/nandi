import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MoreHorizontal, Play, Zap } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  dutyText,
  fireView,
  whenText,
} from '@/components/automations/automations-model'
import { SOURCE_ICONS } from '@/components/triggers/source-icons'
import {
  DeleteTriggerDialog,
  RotateSecretButton,
  TestTriggerDialog,
} from '@/components/triggers/TriggerDialogs'
import {
  FireStatusBadge,
  TriggerEndpoint,
} from '@/components/triggers/trigger-ui'
import { TriggerForm } from '@/components/triggers/TriggerForm'
import {
  useTargetName,
  useTrigger,
  useTriggerFires,
  useTriggerFormProblems,
} from '@/hooks/useTriggers'
import { formatRelativeTime } from '@/lib/format-time'
import { setTriggerEnabled, updateTrigger } from '@/lib/trigger-actions'
import {
  fromTrigger,
  SOURCE_TYPES,
  toInput,
  type TriggerFormState,
} from '@/lib/trigger-form'
import type {
  Trigger,
  TriggerFire,
  TriggerSourceType,
} from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

/** One trigger: its settings and its firings, each linking to its Request. */
export function TriggerDetailView({ triggerId }: { triggerId: string }) {
  const { t, i18n } = useTranslation()
  const { data: trigger, isLoading, error } = useTrigger(triggerId)
  // A just-generated secret (create or rotate); never fetched again
  const [secret, setSecret] = useState<string | null>(null)
  const [testing, setTesting] = useState(false)
  const [deleting, setDeleting] = useState(false)

  if (isLoading) {
    return (
      <div className="flex flex-col gap-4 p-6">
        <Skeleton className="h-10 w-1/2" />
        <Skeleton className="h-64" />
      </div>
    )
  }
  if (error || !trigger) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Zap />
          </EmptyMedia>
          <EmptyTitle>{t('triggers.notFound')}</EmptyTitle>
          <EmptyDescription>
            {t('triggers.notFoundDescription')}
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button
            onClick={() =>
              useNavigationStore
                .getState()
                .navigate({ kind: 'automations', tab: 'triggers' })
            }
          >
            {t('automations.backToAutomations')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={
          <span className="flex size-8 items-center justify-center rounded-md border bg-muted/40 [&_svg]:size-4">
            {SOURCE_ICONS[trigger.sourceType]}
          </span>
        }
        title={trigger.name}
        subtitle={
          <TriggerSubtitle trigger={trigger} language={i18n.language} />
        }
        actions={
          <>
            <label className="flex items-center gap-2 text-sm">
              <Switch
                checked={trigger.enabled}
                onCheckedChange={enabled =>
                  void setTriggerEnabled(trigger.id, enabled)
                }
              />
              {t(trigger.enabled ? 'triggers.enabled' : 'triggers.notFiring')}
            </label>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setTesting(true)}
            >
              <Play />
              {t('triggers.test.open')}
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={t('triggers.rowMenu', { name: trigger.name })}
                >
                  <MoreHorizontal />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  variant="destructive"
                  onSelect={() => setDeleting(true)}
                >
                  {t('triggers.deleteMenu')}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      />
      <TestTriggerDialog
        trigger={trigger}
        open={testing}
        onOpenChange={setTesting}
      />
      <DeleteTriggerDialog
        trigger={trigger}
        open={deleting}
        onOpenChange={setDeleting}
      />
      <Tabs defaultValue="settings" className="min-h-0 flex-1 gap-0">
        <TabsList className="mx-6 mt-3">
          <TabsTrigger value="settings">
            {t('triggers.tabs.settings')}
          </TabsTrigger>
          <TabsTrigger value="firings">
            {t('triggers.tabs.firings')}
          </TabsTrigger>
        </TabsList>
        <ScrollArea className="min-h-0 flex-1">
          <div className="mx-auto w-full max-w-3xl p-6">
            <TabsContent value="settings" className="flex flex-col gap-6">
              {(trigger.sourceType === 'webhook' ||
                trigger.sourceType === 'device') && (
                <section className="flex flex-col gap-3 rounded-lg border p-4">
                  <TriggerEndpoint trigger={trigger} secret={secret} />
                  <RotateSecretButton trigger={trigger} onSecret={setSecret} />
                </section>
              )}
              <SettingsForm
                key={trigger.updatedAt}
                trigger={trigger}
                onSecret={setSecret}
              />
            </TabsContent>
            <TabsContent value="firings">
              <FiringsTable trigger={trigger} />
            </TabsContent>
          </div>
        </ScrollArea>
      </Tabs>
    </div>
  )
}

/** "Every weekday at 9:00 → runs workflow Customer Onboarding" */
function TriggerSubtitle({
  trigger,
  language,
}: {
  trigger: Trigger
  language: string
}) {
  const { t } = useTranslation()
  const target = useTargetName(trigger)
  const when = whenText(trigger, t, language)
  return (
    <>
      {when.text} →{' '}
      {t(`triggers.runsTarget.${trigger.targetType}`, { name: target })}
    </>
  )
}

/** The trigger's settings, with Revert and Save; leaving with changes asks */
function SettingsForm({
  trigger,
  onSecret,
}: {
  trigger: Trigger
  onSecret: (secret: string) => void
}) {
  const { t } = useTranslation()
  const [form, setForm] = useState<TriggerFormState>(() => fromTrigger(trigger))
  const [saving, setSaving] = useState(false)
  const problems = useTriggerFormProblems(form)
  const dirty =
    JSON.stringify(toInput(form)) !==
    JSON.stringify(toInput(fromTrigger(trigger)))

  useEffect(() => {
    useNavigationStore.getState().setLeaveGuarded(dirty)
  }, [dirty])
  useEffect(
    () => () => useNavigationStore.getState().setLeaveGuarded(false),
    []
  )

  const save = async () => {
    setSaving(true)
    const saved = await updateTrigger(trigger.id, toInput(form))
    setSaving(false)
    if (saved?.secret) onSecret(saved.secret)
  }

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={event => {
        event.preventDefault()
        void save()
      }}
    >
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="trigger-source-type">
          {t('triggers.form.sourceType')}
        </Label>
        <NativeSelect
          id="trigger-source-type"
          value={form.sourceType}
          onChange={event =>
            setForm(current => ({
              ...current,
              sourceType: event.target.value as TriggerSourceType,
            }))
          }
        >
          {SOURCE_TYPES.map(type => (
            <NativeSelectOption key={type} value={type}>
              {t(`triggers.source.${type}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <TriggerForm
        form={form}
        onChange={patch => setForm(current => ({ ...current, ...patch }))}
      />
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={!dirty || saving}
          onClick={() => setForm(fromTrigger(trigger))}
        >
          {t('triggers.revert')}
        </Button>
        <Button
          type="submit"
          disabled={!dirty || saving || problems.length > 0}
        >
          {t('triggers.save')}
        </Button>
      </div>
    </form>
  )
}

/** What each firing did; a started one opens its Request */
function FiringsTable({ trigger }: { trigger: Trigger }) {
  const { t, i18n } = useTranslation()
  const { data: fires, isLoading } = useTriggerFires(trigger.id)
  const [selected, setSelected] = useState<string | null>(null)

  if (isLoading) return <Skeleton className="h-32" />
  if (!fires || fires.length === 0) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        {t('triggers.noFires')}
      </p>
    )
  }
  const shown = fires.find(fire => fire.id === selected)

  return (
    <div className="flex flex-col gap-4">
      <div className="overflow-x-auto rounded-lg border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40 text-xs text-muted-foreground">
            <tr className="[&_th]:px-3 [&_th]:py-2 [&_th]:text-start [&_th]:font-medium">
              <th>{t('triggers.fireColumns.firedAt')}</th>
              <th>{t('triggers.fireColumns.result')}</th>
              <th>{t('triggers.fireColumns.request')}</th>
            </tr>
          </thead>
          <tbody>
            {fires.map(fire => (
              <FiringRow
                key={fire.id}
                fire={fire}
                trigger={trigger}
                selected={fire.id === selected}
                onSelect={() =>
                  setSelected(current => (current === fire.id ? null : fire.id))
                }
                language={i18n.language}
              />
            ))}
          </tbody>
        </table>
      </div>
      {shown && (
        <section className="flex flex-col gap-2">
          <p className="text-sm font-medium">{t('triggers.firingPayload')}</p>
          <pre
            dir="ltr"
            className="max-h-96 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs"
          >
            {JSON.stringify(shown.payload, null, 2)}
          </pre>
        </section>
      )}
    </div>
  )
}

function FiringRow({
  fire,
  trigger,
  selected,
  onSelect,
  language,
}: {
  fire: TriggerFire
  trigger: Trigger
  selected: boolean
  onSelect: () => void
  language: string
}) {
  const { t } = useTranslation()
  const view = fireView(fire, trigger)
  const target = useTargetName(trigger)
  const duty = dutyText(fire, t)

  return (
    <tr
      aria-selected={selected}
      className={cn(
        'cursor-pointer border-t hover:bg-accent/40 [&_td]:px-3 [&_td]:py-2',
        selected && 'bg-accent/60'
      )}
      onClick={onSelect}
    >
      <td
        className="text-xs whitespace-nowrap"
        title={new Date(fire.firedAt).toLocaleString(language)}
      >
        {formatRelativeTime(fire.firedAt, language)}
      </td>
      <td className="text-xs">
        <span className="flex flex-wrap items-center gap-2">
          <FireStatusBadge fire={fire} />
          {fire.error && (
            <span className="text-muted-foreground" title={fire.error}>
              {fire.error}
            </span>
          )}
        </span>
        {duty && <span className="mt-1 block text-foreground">{duty}</span>}
      </td>
      <td className="text-xs">
        {view ? (
          <Button
            size="sm"
            variant="link"
            className="h-auto p-0 text-xs"
            onClick={event => {
              event.stopPropagation()
              useNavigationStore.getState().navigate(view)
            }}
          >
            {t('triggers.openRequest', { name: target })}
          </Button>
        ) : (
          <span className="text-muted-foreground">—</span>
        )}
      </td>
    </tr>
  )
}
