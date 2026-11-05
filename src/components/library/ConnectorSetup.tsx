import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ExternalLink, FolderOpen, FolderPlus, X } from 'lucide-react'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import {
  useConfigureConnector,
  useConnectorAction,
  useFolderUsers,
  useSetConnectorSecret,
  type ConnectorDetails,
  type ConnectorSecret,
} from '@/hooks/useConnectors'
import type { JsonValue } from '@/lib/bindings'
import { client, isDesktop } from '@/lib/client'
import { GithubSignInPanel } from '@/components/github/GithubSignInPanel'
import { startBlocked } from './library-model'
import {
  folderConfig,
  folderIdFrom,
  folderProblems,
  foldersOf,
  type FolderAccess,
  type SharedFolder,
} from './shared-folders'

/** Where to get a key, by secret (the hint becomes a link) */
const KEY_LINKS: Record<string, string> = {
  'github/token': 'https://github.com/settings/personal-access-tokens/new',
}

/**
 * A connector's Setup section (§4.5): editable settings, write-only
 * secrets (Save / Replace / Clear, which asks first), and
 * Start, which stays disabled with the reason until everything is set.
 */
export function ConnectorSetup({ connector }: { connector: ConnectorDetails }) {
  const { t } = useTranslation()
  if (!connector.status.installed) {
    return (
      <p className="text-sm text-muted-foreground">
        {t('library.setup.installFirst')}
      </p>
    )
  }
  const secrets = connector.secrets ?? []
  return (
    <div className="space-y-4">
      {connector.id === 'filesystem' ? (
        <SharedFoldersField connector={connector} />
      ) : (
        <ConfigFields connector={connector} />
      )}
      {connector.id === 's3_backup' && (
        <SharedFoldersField connector={connector} backup />
      )}
      {secrets.map(secret => (
        <SecretField key={secret.key} connector={connector} secret={secret} />
      ))}
      {connector.id === 'github' && <GithubSignInPanel showSignOut={false} />}
      <StartRow connector={connector} />
    </div>
  )
}

/** The text and on/off settings, which `ConfigFields` edits */
function editableFields(config: Record<string, unknown>) {
  return Object.entries(config).filter(
    ([, value]) => typeof value === 'string' || typeof value === 'boolean'
  )
}

/** The settings a connector reads, saved to the repo's connector list */
function ConfigFields({ connector }: { connector: ConnectorDetails }) {
  const { t } = useTranslation()
  const configure = useConfigureConnector()
  const saved = connector.config ?? {}
  // The filesystem connector always needs its folder, even before it's set
  const fields =
    connector.id === 'filesystem' && !('path' in saved)
      ? { ...saved, path: '' }
      : saved
  const [draft, setDraft] = useState<Record<string, unknown>>(fields)
  const entries = editableFields(draft)
  if (entries.length === 0) return null
  const dirty =
    JSON.stringify(entries) !== JSON.stringify(editableFields(fields))

  // Only these fields: the rest of the config (a folder list) stays as saved
  const save = () =>
    configure.mutate(
      {
        id: connector.id,
        config: {
          ...saved,
          ...Object.fromEntries(entries),
        } as Record<string, JsonValue>,
      },
      {
        onSuccess: () =>
          toast.success(
            connector.status.running
              ? t('library.setup.savedRestart')
              : t('library.setup.saved')
          ),
        onError: error => toast.error(error.message),
      }
    )

  const browse = async (key: string) => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const chosen = await open({ directory: true, multiple: false })
    if (typeof chosen === 'string') setDraft(d => ({ ...d, [key]: chosen }))
  }

  return (
    <div className="space-y-3">
      {entries.map(([key, value]) => {
        const id = `connector-${connector.id}-${key}`
        const label = t(`library.setup.field.${key}`, { defaultValue: key })
        return typeof value === 'boolean' ? (
          <div key={key} className="flex items-center justify-between gap-2">
            <Label htmlFor={id}>{label}</Label>
            <Switch
              id={id}
              checked={value}
              onCheckedChange={next => setDraft(d => ({ ...d, [key]: next }))}
            />
          </div>
        ) : (
          <div key={key} className="space-y-1">
            <Label htmlFor={id}>{label}</Label>
            <div className="flex gap-2">
              <Input
                id={id}
                dir="ltr"
                value={String(value)}
                onChange={event =>
                  setDraft(d => ({ ...d, [key]: event.target.value }))
                }
              />
              {key === 'path' && isDesktop() && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => void browse(key)}
                >
                  <FolderOpen />
                  {t('library.setup.browse')}
                </Button>
              )}
            </div>
          </div>
        )
      })}
      <Button
        size="sm"
        variant="outline"
        disabled={!dirty || configure.isPending}
        onClick={save}
      >
        {configure.isPending && <Spinner />}
        {t('library.setup.saveSettings')}
      </Button>
    </div>
  )
}

/** A native folder picker; `null` when cancelled or not on desktop */
async function pickFolder(): Promise<string | null> {
  if (!isDesktop()) return null
  const { open } = await import('@tauri-apps/plugin-dialog')
  const chosen = await open({ directory: true, multiple: false })
  return typeof chosen === 'string' ? chosen : null
}

/**
 * The folders agents can be given, each read only or read and write. An
 * agent gets only the folders its page (or skill.md) names. With `backup`
 * (S3 Backup): the folders backed up, read only.
 */
function SharedFoldersField({
  connector,
  backup = false,
}: {
  connector: ConnectorDetails
  backup?: boolean
}) {
  const { t, i18n } = useTranslation()
  const configure = useConfigureConnector()
  const users = useFolderUsers(connector.id)
  const [removing, setRemoving] = useState<number | null>(null)
  const usersOf = (id: string) =>
    users.data?.find(u => u.folder === id)?.agents ?? []
  const names = (agents: string[]) =>
    new Intl.ListFormat(i18n.language, { type: 'conjunction' }).format(agents)
  const remove = (index: number) =>
    setDraft(rows => rows.filter((_, i) => i !== index))
  const saved = foldersOf(connector.config)
  const [draft, setDraft] = useState<SharedFolder[]>(saved)
  const problems = folderProblems(draft)
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved)
  const update = (index: number, change: Partial<SharedFolder>) =>
    setDraft(rows =>
      rows.map((row, i) => (i === index ? { ...row, ...change } : row))
    )

  const add = async () => {
    const path = (await pickFolder()) ?? ''
    setDraft(rows => [
      ...rows,
      {
        id: folderIdFrom(
          path || 'folder',
          rows.map(r => r.id)
        ),
        path,
        access: 'read',
      },
    ])
  }

  const save = () =>
    configure.mutate(
      {
        id: connector.id,
        config: { ...(connector.config ?? {}), ...folderConfig(draft) },
      },
      {
        onSuccess: () =>
          toast.success(
            connector.status.running
              ? t('library.setup.savedRestart')
              : t('library.setup.saved')
          ),
        onError: error => toast.error(error.message),
      }
    )

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <Label>
          {t(backup ? 'library.backupFolders.title' : 'library.folders.title')}
        </Label>
        <p className="text-xs text-muted-foreground">
          {t(
            backup
              ? 'library.backupFolders.description'
              : 'library.folders.description'
          )}
        </p>
      </div>
      {draft.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t('library.folders.empty')}
        </p>
      )}
      <ul className="space-y-2">
        {draft.map((folder, index) => {
          const problem = problems[index]
          const base = `folder-${index}`
          return (
            <li key={index} className="space-y-1.5 rounded-md border p-2">
              <div className="flex gap-2">
                <Input
                  aria-label={t('library.folders.id')}
                  dir="ltr"
                  className="w-32 font-mono text-xs"
                  value={folder.id}
                  onChange={event => update(index, { id: event.target.value })}
                />
                {!backup && (
                  <NativeSelect
                    aria-label={t('library.folders.access')}
                    value={folder.access}
                    onChange={event =>
                      update(index, {
                        access: event.target.value as FolderAccess,
                      })
                    }
                  >
                    <NativeSelectOption value="read">
                      {t('library.folders.read')}
                    </NativeSelectOption>
                    <NativeSelectOption value="write">
                      {t('library.folders.write')}
                    </NativeSelectOption>
                  </NativeSelect>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t('library.folders.remove', { id: folder.id })}
                  // A folder agents use asks first
                  onClick={() =>
                    usersOf(folder.id).length > 0
                      ? setRemoving(index)
                      : remove(index)
                  }
                >
                  <X />
                </Button>
              </div>
              <div className="flex gap-2">
                <Input
                  id={`${base}-path`}
                  aria-label={t('library.folders.path')}
                  dir="ltr"
                  value={folder.path}
                  placeholder={t('library.folders.pathPlaceholder')}
                  onChange={event =>
                    update(index, { path: event.target.value })
                  }
                />
                {isDesktop() && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      void pickFolder().then(path => {
                        if (path) update(index, { path })
                      })
                    }
                  >
                    <FolderOpen />
                    {t('library.setup.browse')}
                  </Button>
                )}
              </div>
              {problem && (
                <p className="text-xs text-destructive">
                  {t(`library.folders.problem.${problem}`)}
                </p>
              )}
              {!backup && users.data && saved.some(f => f.id === folder.id) && (
                <p
                  className="text-xs text-muted-foreground"
                  data-testid="folder-users"
                >
                  {usersOf(folder.id).length > 0
                    ? t('library.folders.usedBy', {
                        agents: names(usersOf(folder.id)),
                      })
                    : t('library.folders.unused')}
                </p>
              )}
            </li>
          )
        })}
      </ul>
      <AlertDialog
        open={removing !== null}
        onOpenChange={open => {
          if (!open) setRemoving(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('library.folders.removeTitle', {
                id: removing === null ? '' : draft[removing]?.id,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('library.folders.removeInUse', {
                agents: names(
                  removing === null ? [] : usersOf(draft[removing]?.id ?? '')
                ),
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('library.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (removing !== null) remove(removing)
                setRemoving(null)
              }}
            >
              {t('library.folders.removeConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <div className="flex gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => void add()}
        >
          <FolderPlus />
          {t('library.folders.add')}
        </Button>
        <Button
          size="sm"
          disabled={
            !dirty || configure.isPending || problems.some(p => p !== null)
          }
          onClick={save}
        >
          {configure.isPending && <Spinner />}
          {t('library.setup.saveSettings')}
        </Button>
      </div>
    </div>
  )
}

/** Write only: a saved value is shown as dots and never read back */
function SecretField({
  connector,
  secret,
}: {
  connector: ConnectorDetails
  secret: ConnectorSecret
}) {
  const { t } = useTranslation()
  const setSecret = useSetConnectorSecret()
  const [replacing, setReplacing] = useState(false)
  const [draft, setDraft] = useState('')
  const [clearing, setClearing] = useState(false)
  const id = `connector-${connector.id}-secret-${secret.key}`
  const label = t(`library.setup.secret.${connector.id}.${secret.key}`, {
    defaultValue: secret.key,
  })
  const link = KEY_LINKS[`${connector.id}/${secret.key}`]

  const save = (value: string) =>
    setSecret.mutate(
      { id: connector.id, key: secret.key, value },
      {
        onSuccess: () => {
          setDraft('')
          setReplacing(false)
          toast.success(
            value
              ? t('library.setup.secretSaved')
              : t('library.setup.secretRemoved')
          )
        },
        onError: error => toast.error(error.message),
      }
    )

  const editing = !secret.isSet || replacing

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-2">
        <Label htmlFor={id}>{label}</Label>
        <Badge variant="outline" className="text-muted-foreground">
          {secret.isSet ? t('library.setup.saved') : t('library.setup.notSet')}
        </Badge>
      </div>
      {editing ? (
        <form
          className="flex gap-2"
          onSubmit={event => {
            event.preventDefault()
            if (draft.trim()) save(draft)
          }}
        >
          <Input
            id={id}
            type="password"
            dir="ltr"
            autoComplete="off"
            spellCheck={false}
            value={draft}
            onChange={event => setDraft(event.target.value)}
            placeholder={t('library.setup.pasteKey')}
          />
          <Button
            type="submit"
            size="sm"
            disabled={!draft.trim() || setSecret.isPending}
          >
            {setSecret.isPending && <Spinner />}
            {t('library.setup.save')}
          </Button>
        </form>
      ) : (
        <div className="flex gap-2">
          <Input
            id={id}
            disabled
            value="••••••••••••"
            aria-label={label}
            className="font-mono"
          />
          <Button
            size="sm"
            variant="outline"
            onClick={() => setReplacing(true)}
          >
            {t('library.setup.replace')}
          </Button>
          <Button size="sm" variant="outline" onClick={() => setClearing(true)}>
            {t('library.setup.clear')}
          </Button>
        </div>
      )}
      {link && (
        <button
          type="button"
          className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:underline"
          onClick={() => void client.openExternal(link)}
        >
          {t('library.setup.getKey', { host: new URL(link).host })}
          <ExternalLink aria-hidden className="size-3" />
        </button>
      )}
      <AlertDialog open={clearing} onOpenChange={setClearing}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('library.setup.clearTitle', { name: label })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('library.setup.clearDescription', {
                connector: connector.name,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('library.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => save('')}>
              {t('library.setup.clearConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** Start, disabled with the reason until every secret and sign-in is in */
function StartRow({ connector }: { connector: ConnectorDetails }) {
  const { t } = useTranslation()
  const action = useConnectorAction()
  const blocked = startBlocked(connector)
  const running = connector.status.running

  return (
    <div className="space-y-1 border-t pt-3">
      <Button
        size="sm"
        variant={running ? 'outline' : 'default'}
        disabled={action.isPending || (!running && blocked !== null)}
        onClick={() =>
          action.mutate(
            { id: connector.id, action: running ? 'stop' : 'start' },
            { onError: error => toast.error(error.message) }
          )
        }
      >
        {action.isPending && <Spinner />}
        {running ? t('library.action.stop') : t('library.action.start')}
      </Button>
      {!running && blocked !== null && (
        <p
          className="text-xs text-muted-foreground"
          data-testid="start-blocked"
        >
          {blocked === 'setup' && connector.setupHint
            ? t('library.setup.blockedBy', { reason: connector.setupHint })
            : t(`library.setup.blocked.${blocked}`)}
        </p>
      )}
      {connector.status.running && !connector.status.healthy && (
        <p className="text-xs text-destructive">
          {connector.status.error ?? t('library.pill.problem')}
        </p>
      )}
    </div>
  )
}
