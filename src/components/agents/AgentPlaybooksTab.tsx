/**
 * The agent page's Playbooks tab: the agent's skill files
 * (`skills/<name>.md`), each with when to use it. The list on one side,
 * the open playbook on the other, edited like skill.md. An installed
 * agent's playbooks are read-only. Rules live in `playbooks-model.ts`.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { BookOpen, Lock, Plus, Trash2 } from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import {
  useDeletePlaybook,
  usePlaybooks,
  useSavePlaybook,
} from '@/hooks/usePlaybooks'
import type { AgentPlaybooks, Playbook } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import type { AgentInfo } from '@/types/agent'
import {
  EMPTY_DRAFT,
  MAX_PLAYBOOK_WORDS,
  draftIssues,
  draftOf,
  keepSelection,
  playbookNameFrom,
  sameDraft,
  toInput,
  wordCount,
  type PlaybookDraft,
  type PlaybookSelection,
} from './playbooks-model'

export function AgentPlaybooksTab({
  agent,
  onDirtyChange,
}: {
  agent: AgentInfo
  onDirtyChange: (dirty: boolean) => void
}) {
  const { t } = useTranslation()
  const playbooks = usePlaybooks(agent.id)

  if (playbooks.isPending) {
    return (
      <div className="mx-auto w-full max-w-5xl space-y-3 p-4">
        <Skeleton className="h-16" />
        <Skeleton className="h-64" />
      </div>
    )
  }
  if (playbooks.isError) {
    return (
      <div className="mx-auto w-full max-w-5xl p-4">
        <Alert variant="destructive">
          <AlertDescription>
            {t('agentPage.playbooks.loadError', {
              error: playbooks.error.message,
            })}
          </AlertDescription>
        </Alert>
      </div>
    )
  }
  return (
    <PlaybooksPanel
      agentId={agent.id}
      data={playbooks.data}
      onDirtyChange={onDirtyChange}
    />
  )
}

function PlaybooksPanel({
  agentId,
  data,
  onDirtyChange,
}: {
  agentId: string
  data: AgentPlaybooks
  onDirtyChange: (dirty: boolean) => void
}) {
  const { t } = useTranslation()
  const names = data.playbooks.map(p => p.name)
  const [chosen, setChosen] = useState<PlaybookSelection>({ kind: 'none' })
  const [dirty, setDirty] = useState(false)
  const [pending, setPending] = useState<PlaybookSelection | null>(null)
  const selection = keepSelection(chosen, names)
  const open =
    selection.kind === 'existing'
      ? data.playbooks.find(p => p.name === selection.name)
      : undefined

  const reportDirty = (next: boolean) => {
    setDirty(next)
    onDirtyChange(next)
  }
  const choose = (next: PlaybookSelection) => {
    if (dirty) {
      setPending(next)
      return
    }
    setChosen(next)
  }

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-4 p-4">
      <p className="text-sm text-muted-foreground">
        {t('agentPage.playbooks.intro')}
      </p>
      {data.readOnly && (
        <Alert>
          <Lock />
          <AlertDescription>
            {t('agentPage.playbooks.readOnly')}
          </AlertDescription>
        </Alert>
      )}
      {data.problems.length > 0 && (
        <Alert variant="destructive">
          <AlertDescription className="flex flex-col gap-1">
            <span>{t('agentPage.playbooks.problems')}</span>
            {data.problems.map(problem => (
              <span key={problem.file} className="font-mono text-xs">
                {problem.file}: {problem.error}
              </span>
            ))}
          </AlertDescription>
        </Alert>
      )}

      <div className="grid gap-4 md:grid-cols-[minmax(0,16rem)_minmax(0,1fr)]">
        <nav
          aria-label={t('agentPage.playbooks.listLabel')}
          className="flex flex-col gap-1"
        >
          {data.playbooks.map(playbook => (
            <PlaybookRow
              key={playbook.name}
              playbook={playbook}
              selected={
                selection.kind === 'existing' &&
                selection.name === playbook.name
              }
              onSelect={() => choose({ kind: 'existing', name: playbook.name })}
            />
          ))}
          {data.playbooks.length === 0 && (
            <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
              {t('agentPage.playbooks.empty')}
            </p>
          )}
          {!data.readOnly && (
            <Button
              variant="outline"
              size="sm"
              className="mt-1 self-start"
              onClick={() => choose({ kind: 'new' })}
            >
              <Plus />
              {t('agentPage.playbooks.add')}
            </Button>
          )}
        </nav>

        {selection.kind === 'new' && (
          <PlaybookEditor
            key="new"
            agentId={agentId}
            playbook={null}
            names={names}
            readOnly={false}
            onDirtyChange={reportDirty}
            onSaved={name => setChosen({ kind: 'existing', name })}
            onDeleted={() => setChosen({ kind: 'none' })}
          />
        )}
        {open && (
          <PlaybookEditor
            key={`${open.name}\n${open.body}\n${open.description}\n${open.when}`}
            agentId={agentId}
            playbook={open}
            names={names}
            readOnly={data.readOnly}
            onDirtyChange={reportDirty}
            onSaved={name => setChosen({ kind: 'existing', name })}
            onDeleted={() => setChosen({ kind: 'none' })}
          />
        )}
      </div>

      <AlertDialog
        open={pending !== null}
        onOpenChange={isOpen => !isOpen && setPending(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('agentPage.settings.discardTitle')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('agentPage.settings.discardDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('agentPage.settings.keepEditing')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                reportDirty(false)
                if (pending) setChosen(pending)
                setPending(null)
              }}
            >
              {t('agentPage.settings.discard')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function PlaybookRow({
  playbook,
  selected,
  onSelect,
}: {
  playbook: Playbook
  selected: boolean
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      aria-current={selected ? 'true' : undefined}
      onClick={onSelect}
      className={cn(
        'flex flex-col gap-0.5 rounded-md border px-3 py-2 text-start transition-colors hover:bg-accent',
        selected && 'border-primary bg-accent'
      )}
    >
      <span className="flex items-center gap-1.5 font-mono text-xs font-medium">
        <BookOpen className="size-3.5 shrink-0 text-muted-foreground" />
        {playbook.name}
      </span>
      <span className="line-clamp-2 text-xs">{playbook.description}</span>
      <span className="line-clamp-1 text-xs text-muted-foreground">
        {playbook.when}
      </span>
    </button>
  )
}

function PlaybookEditor({
  agentId,
  playbook,
  names,
  readOnly,
  onDirtyChange,
  onSaved,
  onDeleted,
}: {
  agentId: string
  /** `null`: a new one */
  playbook: Playbook | null
  names: string[]
  readOnly: boolean
  onDirtyChange: (dirty: boolean) => void
  onSaved: (name: string) => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const save = useSavePlaybook(agentId)
  const remove = useDeletePlaybook(agentId)
  const saved = playbook ? draftOf(playbook) : EMPTY_DRAFT
  const [draft, setDraft] = useState<PlaybookDraft>(saved)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const previous = playbook?.name ?? null
  const dirty = !sameDraft(draft, saved)
  const issues = draftIssues(draft, names, previous)
  const words = wordCount(draft.body)
  const fail = (error: unknown) =>
    toast.error(error instanceof Error ? error.message : String(error))

  const update = (patch: Partial<PlaybookDraft>) => {
    const next = { ...draft, ...patch }
    setDraft(next)
    onDirtyChange(!sameDraft(next, saved))
  }
  const field = (name: keyof PlaybookDraft) => `playbook-${name}`
  const issueText = (...kinds: string[]) => {
    const found = issues.find(issue => kinds.includes(issue))
    return dirty && found ? t(`agentPage.playbooks.issue.${found}`) : null
  }

  return (
    <section className="flex min-w-0 flex-col gap-3">
      <fieldset disabled={readOnly} className="flex flex-col gap-3">
        <div className="grid gap-3 sm:grid-cols-2">
          <Field
            id={field('name')}
            label={t('agentPage.playbooks.name')}
            hint={t('agentPage.playbooks.nameHint')}
            error={issueText('nameRequired', 'nameFormat', 'nameTaken')}
          >
            <Input
              id={field('name')}
              className="font-mono text-xs"
              value={draft.name}
              placeholder="pay-a-bill"
              onChange={event => update({ name: event.target.value })}
              onBlur={() => {
                const name = playbookNameFrom(draft.name)
                if (name && name !== draft.name) update({ name })
              }}
            />
          </Field>
          <Field
            id={field('when')}
            label={t('agentPage.playbooks.when')}
            hint={t('agentPage.playbooks.whenHint')}
            error={issueText('whenRequired')}
          >
            <Input
              id={field('when')}
              value={draft.when}
              onChange={event => update({ when: event.target.value })}
            />
          </Field>
        </div>
        <Field
          id={field('description')}
          label={t('agentPage.playbooks.description')}
          error={issueText('descriptionRequired', 'oneLine', 'lineTooLong')}
        >
          <Input
            id={field('description')}
            value={draft.description}
            onChange={event => update({ description: event.target.value })}
          />
        </Field>
        <Field
          id={field('body')}
          label={t('agentPage.playbooks.body')}
          error={issueText('bodyRequired', 'tooLong')}
        >
          <Textarea
            id={field('body')}
            rows={18}
            className="font-mono text-xs"
            value={draft.body}
            onChange={event => update({ body: event.target.value })}
          />
        </Field>
        <p
          className={cn(
            'text-xs text-muted-foreground',
            words > MAX_PLAYBOOK_WORDS && 'text-destructive'
          )}
        >
          {t('agentPage.playbooks.words', {
            words,
            max: MAX_PLAYBOOK_WORDS,
          })}
        </p>
      </fieldset>

      {!readOnly && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          {playbook ? (
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive"
              disabled={remove.isPending}
              onClick={() => setConfirmDelete(true)}
            >
              <Trash2 />
              {t('agentPage.playbooks.delete')}
            </Button>
          ) : (
            <span />
          )}
          <div className="flex gap-2">
            {dirty && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setDraft(saved)
                  onDirtyChange(false)
                }}
              >
                {t('agentPage.settings.revert')}
              </Button>
            )}
            <Button
              size="sm"
              disabled={!dirty || issues.length > 0 || save.isPending}
              onClick={() =>
                save.mutate(
                  { input: toInput(draft), previous },
                  {
                    onSuccess: result => {
                      onDirtyChange(false)
                      toast.success(t('agentPage.playbooks.saved'))
                      onSaved(result.name)
                    },
                    onError: fail,
                  }
                )
              }
            >
              {save.isPending && <Spinner />}
              {t('agentPage.playbooks.save')}
            </Button>
          </div>
        </div>
      )}

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('agentPage.playbooks.deleteTitle', { name: previous })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('agentPage.playbooks.deleteDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('agentPage.playbooks.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (!previous) return
                remove.mutate(previous, {
                  onSuccess: () => {
                    onDirtyChange(false)
                    toast.success(t('agentPage.playbooks.deleted'))
                    onDeleted()
                  },
                  onError: fail,
                })
              }}
            >
              {t('agentPage.playbooks.delete')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}

function Field({
  id,
  label,
  hint,
  error,
  children,
}: {
  id: string
  label: string
  hint?: string
  error?: string | null
  children: React.ReactNode
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : (
        hint && <p className="text-xs text-muted-foreground">{hint}</p>
      )}
    </div>
  )
}
