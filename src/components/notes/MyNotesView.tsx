import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { FileText, FolderOpen, Plus, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
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
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useAgents } from '@/hooks/useAgents'
import {
  useDeleteNote,
  useMyNotes,
  useNote,
  useSaveNote,
  useSetNotesAgentsCanWrite,
} from '@/hooks/useNotes'
import { formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { MarkdownEditor } from './MarkdownEditor'
import {
  groupNotes,
  newNoteContent,
  newNoteName,
  noteLabel,
} from './notes-model'
import { NoteSuggestions } from './NoteSuggestions'

/** The agent that keeps the notes (built into Nandi) */
const CHIEF_OF_STAFF = 'chief-of-staff'

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error)

/**
 * My notes: the Markdown notes Chief of Staff keeps for the user (role,
 * what they own, deadlines, who to ask…), and answers from. The list on
 * the left, the open note on the right, read or edited in place. The
 * switch lets Chief of Staff update them without asking each time.
 */
export function MyNotesView({ note }: { note?: string }) {
  const { t } = useTranslation()
  const notes = useMyNotes()
  const [selected, setSelected] = useState<string | null>(note ?? null)
  const [dirty, setDirty] = useState(false)
  const [pending, setPending] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const save = useSaveNote()

  const list = notes.data?.notes ?? []
  // The open note: the one chosen, else the first
  const current =
    selected && list.some(n => n.name === selected)
      ? selected
      : (list[0]?.name ?? null)

  useEffect(() => {
    useNavigationStore.getState().setLeaveGuarded(dirty)
  }, [dirty])
  useEffect(
    () => () => useNavigationStore.getState().setLeaveGuarded(false),
    []
  )

  const open = (name: string) => {
    if (name === current) return
    if (dirty) setPending(name)
    else setSelected(name)
  }

  const create = (typed: string) => {
    const name = newNoteName(typed)
    if (!name) {
      toast.error(t('notes.badName'))
      return
    }
    if (list.some(n => n.name.toLowerCase() === name.toLowerCase())) {
      setCreating(false)
      open(name)
      return
    }
    save.mutate(
      { name, content: newNoteContent(name) },
      {
        onSuccess: () => {
          setCreating(false)
          setDirty(false)
          setSelected(name)
        },
        onError: error => toast.error(errorText(error)),
      }
    )
  }

  if (notes.isPending) {
    return (
      <div className="flex flex-col gap-3 p-6">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-64 w-full" />
      </div>
    )
  }
  if (notes.isError) {
    return (
      <p className="p-6 text-sm text-destructive">{errorText(notes.error)}</p>
    )
  }
  const mine = notes.data

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-4 border-b px-6 py-4">
        <div className="min-w-0 space-y-1">
          <h1 className="text-lg font-semibold">{t('notes.title')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('notes.description')}
          </p>
          <p
            className="flex items-center gap-1 truncate text-xs text-muted-foreground"
            title={mine.path}
          >
            <FolderOpen aria-hidden className="size-3.5 shrink-0" />
            {mine.path}
          </p>
        </div>
        <div className="flex items-center gap-4">
          <KeepSwitch allowed={mine.agentsCanWrite} />
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus aria-hidden />
            {t('notes.new')}
          </Button>
        </div>
      </header>
      <NoteSuggestions />

      {list.length === 0 && !creating ? (
        <EmptyNotes onNew={() => setCreating(true)} />
      ) : (
        <div className="flex min-h-0 flex-1">
          <nav
            aria-label={t('notes.listLabel')}
            className="flex w-60 shrink-0 flex-col gap-3 overflow-y-auto border-e p-3"
          >
            {creating && (
              <NewNoteInput
                busy={save.isPending}
                onCreate={create}
                onCancel={() => setCreating(false)}
              />
            )}
            {groupNotes(list).map(group => (
              <div key={group.folder} className="flex flex-col gap-0.5">
                {group.folder && (
                  <p className="px-2 pt-1 text-xs font-medium text-muted-foreground">
                    {group.folder}/
                  </p>
                )}
                {group.notes.map(item => (
                  <button
                    key={item.name}
                    type="button"
                    onClick={() => open(item.name)}
                    aria-current={item.name === current ? 'page' : undefined}
                    className={cn(
                      'flex items-center gap-2 rounded-md px-2 py-1.5 text-start text-sm hover:bg-accent',
                      item.name === current && 'bg-accent font-medium'
                    )}
                  >
                    <FileText
                      aria-hidden
                      className="size-4 shrink-0 text-muted-foreground"
                    />
                    <span className="truncate">{noteLabel(item)}</span>
                  </button>
                ))}
              </div>
            ))}
          </nav>
          <section className="flex min-w-0 flex-1 flex-col">
            {current && (
              <NotePanel
                key={current}
                name={current}
                updatedAt={
                  list.find(n => n.name === current)?.updatedAt ?? null
                }
                onDirty={setDirty}
                onDeleted={() => {
                  setDirty(false)
                  setSelected(null)
                }}
              />
            )}
          </section>
        </div>
      )}

      <AlertDialog
        open={pending !== null}
        onOpenChange={openDialog => !openDialog && setPending(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('notes.discard.title')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('notes.discard.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('notes.discard.keep')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setDirty(false)
                setSelected(pending)
                setPending(null)
              }}
            >
              {t('notes.discard.discard')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** "Let Chief of Staff keep my notes up to date" */
function KeepSwitch({ allowed }: { allowed: boolean }) {
  const { t } = useTranslation()
  const set = useSetNotesAgentsCanWrite()
  return (
    <div className="flex items-center gap-2">
      <Switch
        id="notes-agents-can-write"
        checked={allowed}
        disabled={set.isPending}
        onCheckedChange={allow =>
          set.mutate(allow, {
            onError: error => toast.error(errorText(error)),
          })
        }
      />
      <Label htmlFor="notes-agents-can-write" className="text-sm font-normal">
        {t('notes.agentsCanWrite')}
      </Label>
    </div>
  )
}

function NewNoteInput({
  busy,
  onCreate,
  onCancel,
}: {
  busy: boolean
  onCreate: (name: string) => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const [name, setName] = useState('')
  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={event => {
        event.preventDefault()
        onCreate(name)
      }}
    >
      <Input
        autoFocus
        value={name}
        onChange={event => setName(event.target.value)}
        onKeyDown={event => event.key === 'Escape' && onCancel()}
        placeholder={t('notes.newPlaceholder')}
        aria-label={t('notes.newPlaceholder')}
        className="h-8 text-sm"
      />
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={busy || !name.trim()}>
          {t('notes.create')}
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onCancel}>
          {t('notes.cancel')}
        </Button>
      </div>
    </form>
  )
}

/** One note: read it, or edit the Markdown and save */
function NotePanel({
  name,
  updatedAt,
  onDirty,
  onDeleted,
}: {
  name: string
  updatedAt: number | null
  onDirty: (dirty: boolean) => void
  onDeleted: () => void
}) {
  const { t, i18n } = useTranslation()
  const note = useNote(name)
  const save = useSaveNote()
  const remove = useDeleteNote()
  const [mode, setMode] = useState<'read' | 'edit'>('read')
  // The text being edited; `null` until the user changes something
  const [draft, setDraft] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const saved = note.data ?? ''
  const text = draft ?? saved
  const dirty = draft !== null && draft !== saved

  useEffect(() => onDirty(dirty), [dirty, onDirty])

  const submit = () => {
    if (!dirty) return
    save.mutate(
      { name, content: text },
      {
        onSuccess: () => {
          setDraft(null)
          toast.success(t('notes.saved', { name }))
        },
        onError: error => toast.error(errorText(error)),
      }
    )
  }

  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-3 border-b px-6 py-2">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{name}</p>
          {updatedAt !== null && updatedAt > 0 && (
            <p className="text-xs text-muted-foreground">
              {t('notes.updated', {
                when: formatRelativeTime(updatedAt, i18n.language),
              })}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <ToggleGroup
            type="single"
            size="sm"
            variant="outline"
            value={mode}
            onValueChange={value => value && setMode(value as 'read' | 'edit')}
          >
            <ToggleGroupItem value="read">{t('notes.read')}</ToggleGroupItem>
            <ToggleGroupItem value="edit">{t('notes.edit')}</ToggleGroupItem>
          </ToggleGroup>
          {dirty && (
            <Button size="sm" disabled={save.isPending} onClick={submit}>
              {t('notes.save')}
            </Button>
          )}
          <Button
            size="icon"
            variant="ghost"
            aria-label={t('notes.delete')}
            onClick={() => setConfirmDelete(true)}
          >
            <Trash2 aria-hidden />
          </Button>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {note.isPending ? (
          <Skeleton className="m-6 h-40" />
        ) : note.isError ? (
          <p className="p-6 text-sm text-destructive">
            {errorText(note.error)}
          </p>
        ) : mode === 'edit' ? (
          <MarkdownEditor
            value={text}
            onChange={setDraft}
            onSave={submit}
            ariaLabel={t('notes.editorLabel', { name })}
            placeholder={t('notes.editorPlaceholder')}
          />
        ) : (
          <div className="max-w-3xl px-6 py-4">
            {text.trim() ? (
              <MarkdownContent content={text} />
            ) : (
              <p className="text-sm text-muted-foreground">
                {t('notes.emptyNote')}
              </p>
            )}
          </div>
        )}
      </div>
      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('notes.deleteConfirm.title', { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('notes.deleteConfirm.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('notes.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                remove.mutate(name, {
                  onSuccess: onDeleted,
                  onError: error => toast.error(errorText(error)),
                })
              }
            >
              {t('notes.delete')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

/** No notes yet: Chief of Staff can write them by getting to know you */
function EmptyNotes({ onNew }: { onNew: () => void }) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  const chief = agents.find(agent => agent.id === CHIEF_OF_STAFF)
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <FileText aria-hidden className="size-8 text-muted-foreground" />
      <p className="text-sm font-medium">{t('notes.empty.title')}</p>
      <p className="max-w-md text-sm text-muted-foreground">
        {t('notes.empty.withChief')}
      </p>
      <div className="flex gap-2">
        {chief && (
          <Button
            onClick={() =>
              useNavigationStore.getState().navigate({
                kind: 'agent',
                agentId: chief.id,
                tab: 'chat',
              })
            }
          >
            {t('notes.empty.openChief')}
          </Button>
        )}
        <Button variant="outline" onClick={onNew}>
          {t('notes.new')}
        </Button>
      </div>
    </div>
  )
}
