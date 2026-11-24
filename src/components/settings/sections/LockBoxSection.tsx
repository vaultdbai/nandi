import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { KeyRound, Lock, LockOpen, Pencil, Plus, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
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
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
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
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { useAgents } from '@/hooks/useAgents'
import {
  useAddKey,
  useDeleteKey,
  useLockBox,
  useLockKey,
  useUnlockKey,
  useUpdateKey,
} from '@/hooks/useLockBox'
import type { KeyKind, KeyView, LockLevel } from '@/lib/bindings'
import { SettingsSectionFrame } from '../SettingsFrame'
import {
  emptyForm,
  formOf,
  formProblem,
  HEADER_PRESETS,
  KEY_KINDS,
  keyWarning,
  LOCK_LEVELS,
  lockKey,
  rulesOf,
  UNLOCK_MINUTES,
  type HeaderPreset,
  type KeyForm,
} from './lock-box-model'

/** Commands reject with the core's error string, not an `Error` */
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/**
 * Settings › Lock box: keys agents can use but never see. Each key is
 * listed by name, kind, sites, agents, lock level and last use; never its
 * value, which goes straight into the system keychain when it is added.
 */
export function LockBoxSection() {
  const { t } = useTranslation()
  const lockBox = useLockBox()
  const [editing, setEditing] = useState<KeyView | 'new' | null>(null)
  return (
    <SettingsSectionFrame
      title={t('settings.section.lockBox')}
      description={t('lockBox.description')}
      actions={
        <Button size="sm" onClick={() => setEditing('new')}>
          <Plus />
          {t('lockBox.add')}
        </Button>
      }
    >
      <p className="text-sm text-muted-foreground">
        {t('lockBox.whatAgentsSee')}
      </p>
      {lockBox.isPending && <Skeleton className="h-32 w-full" />}
      {lockBox.isError && (
        <Alert variant="destructive">
          <AlertDescription>{t('lockBox.loadFailed')}</AlertDescription>
        </Alert>
      )}
      {lockBox.isSuccess && lockBox.data.systemUnlock === null && (
        <Alert>
          <AlertDescription>{t('lockBox.noSystemUnlock')}</AlertDescription>
        </Alert>
      )}
      {lockBox.isSuccess && lockBox.data.keys.length === 0 && (
        <p className="text-sm text-muted-foreground">{t('lockBox.none')}</p>
      )}
      {lockBox.data?.keys.map(key => (
        <KeyCard
          key={key.name}
          entry={key}
          systemUnlock={lockBox.data.systemUnlock}
          onEdit={() => setEditing(key)}
        />
      ))}
      {editing !== null && (
        <KeyDialog
          entry={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
        />
      )}
    </SettingsSectionFrame>
  )
}

function KeyCard({
  entry,
  systemUnlock,
  onEdit,
}: {
  entry: KeyView
  systemUnlock: string | null
  onEdit: () => void
}) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  const unlock = useUnlockKey()
  const lock = useLockKey()
  const nameOf = (id: string) => agents.find(a => a.id === id)?.name ?? id
  const warning = keyWarning(entry)
  const rules = entry.rules
  const locked = rules.lock === 'locked'
  return (
    <section
      className="flex flex-col gap-2 rounded-lg border p-4"
      aria-label={entry.name}
    >
      <div className="flex flex-wrap items-start gap-3">
        <KeyRound className="mt-0.5 size-5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <h3 className="font-mono text-sm font-semibold">{entry.name}</h3>
          <div className="mt-1 flex flex-wrap gap-1.5">
            <Badge variant="secondary">{t(`lockBox.kind.${rules.kind}`)}</Badge>
            <Badge variant="outline">{t(lockKey(rules.lock))}</Badge>
          </div>
        </div>
        {locked &&
          systemUnlock !== null &&
          (entry.unlockedUntil === null ? (
            <Button
              size="sm"
              variant="outline"
              disabled={unlock.isPending}
              onClick={() =>
                unlock.mutate(entry.name, {
                  onError: error =>
                    toast.error(t('lockBox.unlockFailed'), {
                      description: errorText(error),
                    }),
                })
              }
            >
              {unlock.isPending ? <Spinner /> : <LockOpen />}
              {t('lockBox.unlock', { prompt: systemUnlock })}
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              disabled={lock.isPending}
              onClick={() => lock.mutate(entry.name)}
            >
              <Lock />
              {t('lockBox.lockNow')}
            </Button>
          ))}
        <Button
          size="sm"
          variant="outline"
          aria-label={t('lockBox.edit', { name: entry.name })}
          onClick={onEdit}
        >
          <Pencil />
        </Button>
        <DeleteButton name={entry.name} />
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted-foreground">{t('lockBox.sites')}</dt>
        <dd className="font-mono text-xs break-all">
          {rules.hosts.length > 0
            ? rules.hosts.join(', ')
            : t('lockBox.noneSet')}
        </dd>
        <dt className="text-muted-foreground">{t('lockBox.agents')}</dt>
        <dd>
          {rules.agents.length > 0
            ? rules.agents.map(nameOf).join(', ')
            : t('lockBox.noneSet')}
        </dd>
        <dt className="text-muted-foreground">{t('lockBox.lastUse')}</dt>
        <dd>
          {entry.lastUsedAt === null
            ? t('lockBox.neverUsed')
            : t('lockBox.lastUseBy', {
                when: new Date(entry.lastUsedAt).toLocaleString(),
                agent: nameOf(entry.lastUsedBy ?? ''),
                host: entry.lastUsedHost ?? '',
              })}
        </dd>
        {entry.unlockedUntil !== null && (
          <>
            <dt className="text-muted-foreground">{t('lockBox.unlocked')}</dt>
            <dd>
              {t('lockBox.unlockedUntil', {
                when: new Date(entry.unlockedUntil).toLocaleTimeString(),
              })}
            </dd>
          </>
        )}
      </dl>
      {warning && <p className="text-xs text-warning">{t(warning)}</p>}
    </section>
  )
}

function DeleteButton({ name }: { name: string }) {
  const { t } = useTranslation()
  const remove = useDeleteKey()
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        aria-label={t('lockBox.delete', { name })}
        disabled={remove.isPending}
        onClick={() => setConfirming(true)}
      >
        <Trash2 />
      </Button>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('lockBox.deleteTitle', { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('lockBox.deleteDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('lockBox.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                remove.mutate(name, {
                  onSuccess: () =>
                    toast.success(t('lockBox.deleted', { name })),
                  onError: error =>
                    toast.error(t('lockBox.deleteFailed'), {
                      description: errorText(error),
                    }),
                })
              }
            >
              {t('lockBox.deleteConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

/** Add a key (its value typed once), or change a saved key's rules */
function KeyDialog({
  entry,
  onClose,
}: {
  entry: KeyView | null
  onClose: () => void
}) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  const add = useAddKey()
  const update = useUpdateKey()
  const [form, setForm] = useState<KeyForm>(() =>
    entry ? formOf(entry) : emptyForm()
  )
  const [value, setValue] = useState('')
  const adding = entry === null
  const problem = formProblem(form, adding ? value : null)
  const pending = add.isPending || update.isPending
  const set = (patch: Partial<KeyForm>) => setForm({ ...form, ...patch })

  const save = () => {
    const rules = rulesOf(form)
    const done = {
      onSuccess: () => {
        toast.success(t('lockBox.saved', { name: form.name.trim() }))
        onClose()
      },
      onError: (error: unknown) =>
        toast.error(t('lockBox.saveFailed'), { description: errorText(error) }),
    }
    if (adding) {
      add.mutate({ name: form.name.trim(), value, rules }, done)
    } else {
      update.mutate({ name: entry.name, rules }, done)
    }
  }

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {adding
              ? t('lockBox.addTitle')
              : t('lockBox.editTitle', { name: entry.name })}
          </DialogTitle>
          <DialogDescription>{t('lockBox.formDescription')}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-box-name">{t('lockBox.name')}</Label>
            <Input
              id="lock-box-name"
              value={form.name}
              disabled={!adding}
              placeholder="github"
              autoComplete="off"
              onChange={event => set({ name: event.target.value })}
            />
          </div>
          {adding && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="lock-box-value">{t('lockBox.value')}</Label>
              <Input
                id="lock-box-value"
                type="password"
                value={value}
                autoComplete="new-password"
                spellCheck={false}
                onChange={event => setValue(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                {t('lockBox.valueHint')}
              </p>
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-box-kind">{t('lockBox.kindLabel')}</Label>
            <NativeSelect
              id="lock-box-kind"
              value={form.kind}
              onChange={event => set({ kind: event.target.value as KeyKind })}
            >
              {KEY_KINDS.map(kind => (
                <NativeSelectOption key={kind} value={kind}>
                  {t(`lockBox.kind.${kind}`)}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-box-sites">{t('lockBox.sites')}</Label>
            <Textarea
              id="lock-box-sites"
              value={form.hosts}
              rows={2}
              placeholder="api.github.com"
              spellCheck={false}
              onChange={event => set({ hosts: event.target.value })}
            />
            <p className="text-xs text-muted-foreground">
              {t('lockBox.sitesHint')}
            </p>
          </div>
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-1 text-sm font-medium">
              {t('lockBox.agents')}
            </legend>
            {agents.map(agent => {
              const checked = form.agents.includes(agent.id)
              return (
                <label
                  key={agent.id}
                  className="flex items-center gap-2 text-sm"
                >
                  <Checkbox
                    checked={checked}
                    onCheckedChange={next =>
                      set({
                        agents:
                          next === true
                            ? [...form.agents, agent.id]
                            : form.agents.filter(id => id !== agent.id),
                      })
                    }
                  />
                  {agent.name}
                </label>
              )
            })}
          </fieldset>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-box-lock">{t('lockBox.lockLabel')}</Label>
            <NativeSelect
              id="lock-box-lock"
              value={form.lock}
              onChange={event => set({ lock: event.target.value as LockLevel })}
            >
              {LOCK_LEVELS.map(level => (
                <NativeSelectOption key={level} value={level}>
                  {t(lockKey(level))}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            <p className="text-xs text-muted-foreground">
              {t(`lockBox.lockHint.${form.lock}`)}
            </p>
          </div>
          {form.lock === 'locked' && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="lock-box-minutes">{t('lockBox.unlockFor')}</Label>
              <NativeSelect
                id="lock-box-minutes"
                value={String(form.unlockMinutes)}
                onChange={event =>
                  set({ unlockMinutes: Number(event.target.value) })
                }
              >
                {UNLOCK_MINUTES.map(minutes => (
                  <NativeSelectOption key={minutes} value={String(minutes)}>
                    {t('lockBox.minutes', { count: minutes })}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-box-header">{t('lockBox.headerLabel')}</Label>
            <NativeSelect
              id="lock-box-header"
              value={form.header}
              onChange={event =>
                set({ header: event.target.value as HeaderPreset })
              }
            >
              {HEADER_PRESETS.map(preset => (
                <NativeSelectOption key={preset} value={preset}>
                  {t(`lockBox.header.${preset}`)}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            {form.header === 'custom' && (
              <div className="flex gap-2">
                <Input
                  aria-label={t('lockBox.headerName')}
                  placeholder={t('lockBox.headerName')}
                  value={form.customHeader}
                  onChange={event => set({ customHeader: event.target.value })}
                />
                <Input
                  aria-label={t('lockBox.headerPrefix')}
                  placeholder={t('lockBox.headerPrefix')}
                  value={form.customPrefix}
                  onChange={event => set({ customPrefix: event.target.value })}
                />
              </div>
            )}
          </div>
          {problem && form.name.trim() !== '' && (
            <p className="text-xs text-destructive">{t(problem)}</p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t('lockBox.cancel')}
          </Button>
          <Button disabled={problem !== null || pending} onClick={save}>
            {pending && <Spinner />}
            {adding ? t('lockBox.addConfirm') : t('lockBox.save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
