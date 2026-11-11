/**
 * Settings › Backup: the optional git remote for the agents repo. Status,
 * test / push / pull, change or remove, the remote's branches, and merge
 * conflicts left by a pull. Nandi only ever pushes `<prefix>/<slug>`.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import { toast } from 'sonner'
import {
  AlertTriangle,
  ArrowDownToLine,
  ArrowUpFromLine,
  GitBranch,
  Pencil,
  PlugZap,
  RotateCw,
  Trash2,
} from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
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
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import type { RemoteStatus } from '@/lib/bindings'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { RemoteForm } from './RemoteForm'
import { RemoteStatusCard } from './RemoteStatusCard'
import { rawError } from '@/lib/friendly-error'
import { useIdentity } from '@/hooks/useIdentity'
import {
  useClearRemote,
  usePullFromRemote,
  usePushToRemote,
  useRemoteBranches,
  useRemoteStatus,
  useTestRemoteConnection,
} from '@/hooks/useRemote'

export function RemoteSection() {
  const { t } = useTranslation()
  const status = useRemoteStatus()
  const identity = useIdentity()
  const slug = identity.data?.userSlug ?? ''

  if (status.isPending) {
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-9 w-48" />
      </div>
    )
  }
  if (status.isError) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {t('remote.loadFailed', { error: rawError(status.error) })}
      </p>
    )
  }
  if (!status.data.configured) {
    return <EmptyRemote status={status.data} slug={slug} />
  }
  return <ConfiguredRemote status={status.data} slug={slug} />
}

function EmptyRemote({ status, slug }: { status: RemoteStatus; slug: string }) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-col gap-4" data-testid="remote-empty">
      <Empty className="border p-6">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <GitBranch />
          </EmptyMedia>
          <EmptyTitle>{t('remote.empty.title')}</EmptyTitle>
          <EmptyDescription>{t('remote.empty.description')}</EmptyDescription>
        </EmptyHeader>
      </Empty>
      <RemoteForm status={status} slug={slug} />
    </div>
  )
}

function ConfiguredRemote({
  status,
  slug,
}: {
  status: RemoteStatus
  slug: string
}) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState(false)
  const [confirmRemove, setConfirmRemove] = useState(false)
  const [pushConflict, setPushConflict] = useState(false)
  const [confirmPush, setConfirmPush] = useState(false)
  const [confirmPull, setConfirmPull] = useState(false)
  const localChanges = useLocalChanges()
  // Conflicted files from the last pull (the status carries them too)
  const [pullConflicts, setPullConflicts] = useState<string[] | null>(null)
  const test = useTestRemoteConnection()
  const push = usePushToRemote()
  const pull = usePullFromRemote()
  const clear = useClearRemote()

  const runTest = () =>
    test.mutate(undefined, {
      onSuccess: () => toast.success(t('remote.testOk')),
      onError: error =>
        toast.error(t('remote.testFailed', { error: rawError(error) })),
    })

  const runPush = (overwrite: boolean) =>
    push.mutate(overwrite, {
      onSuccess: result => {
        if (result.conflict) {
          setPushConflict(true)
          return
        }
        setPushConflict(false)
        toast.success(
          t('remote.pushed', {
            count: result.pushedCommits,
            branch: result.targetBranch,
          })
        )
      },
      onError: error =>
        toast.error(t('remote.pushFailed', { error: rawError(error) })),
    })

  const runPull = () =>
    pull.mutate(undefined, {
      onSuccess: result => {
        if (result.conflicts) {
          setPullConflicts(result.conflictedFiles)
          return
        }
        setPullConflicts(null)
        toast.success(t('remote.pulled', { count: result.pulledCommits }))
      },
      onError: error =>
        toast.error(t('remote.pullFailed', { error: rawError(error) })),
    })

  const remove = () =>
    clear.mutate(undefined, {
      onSuccess: () => {
        setConfirmRemove(false)
        toast.success(t('remote.removed'))
      },
      onError: error =>
        toast.error(t('remote.removeFailed', { error: rawError(error) })),
    })

  const conflictedFiles =
    pullConflicts ?? (status.conflicts ? status.conflictedFiles : null)

  return (
    <div className="flex flex-col gap-4" data-testid="remote-configured">
      {conflictedFiles && (
        <ConflictBanner
          files={conflictedFiles}
          retrying={pull.isPending}
          onRetry={runPull}
        />
      )}

      <RemoteStatusCard status={status} />

      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={test.isPending}
          onClick={runTest}
        >
          {test.isPending ? <Spinner /> : <PlugZap />}
          {t('remote.actions.test')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={push.isPending}
          onClick={() => setConfirmPush(true)}
        >
          {push.isPending ? <Spinner /> : <ArrowUpFromLine />}
          {t('remote.actions.push')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={pull.isPending}
          onClick={() => (localChanges > 0 ? setConfirmPull(true) : runPull())}
        >
          {pull.isPending ? <Spinner /> : <ArrowDownToLine />}
          {t('remote.actions.pull')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setEditing(value => !value)}
        >
          <Pencil />
          {t('remote.actions.change')}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          onClick={() => setConfirmRemove(true)}
        >
          <Trash2 />
          {t('remote.actions.remove')}
        </Button>
      </div>

      {editing && (
        <div className="rounded-lg border p-4">
          <RemoteForm
            status={status}
            slug={slug}
            onSaved={() => setEditing(false)}
            onCancel={() => setEditing(false)}
          />
        </div>
      )}

      <BranchBrowser
        ownBranch={status.targetBranch}
        pulling={pull.isPending}
        onPull={runPull}
      />

      <AlertDialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('remote.removeDialog.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('remote.removeDialog.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={clear.isPending}
              onClick={event => {
                event.preventDefault()
                remove()
              }}
            >
              {t('remote.removeDialog.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmPush} onOpenChange={setConfirmPush}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('remote.pushConfirm.title')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('remote.pushConfirm.description', {
                remote: status.config?.url ?? '',
                branch: status.targetBranch,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => runPush(false)}>
              {t('remote.actions.push')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmPull} onOpenChange={setConfirmPull}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('remote.pullConfirm.title')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('remote.pullConfirm.description', { count: localChanges })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={runPull}>
              {t('remote.actions.pull')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={pushConflict} onOpenChange={setPushConflict}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('remote.pushConflict.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('remote.pushConflict.description', {
                branch: status.targetBranch,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <p className="flex items-start gap-2 text-sm text-destructive">
            <AlertTriangle className="mt-0.5 size-4 shrink-0" />
            {t('remote.pushConflict.warning', { branch: status.targetBranch })}
          </p>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={push.isPending}
              onClick={event => {
                event.preventDefault()
                runPush(true)
              }}
            >
              {t('remote.pushConflict.overwrite', {
                branch: status.targetBranch,
              })}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/**
 * A pull left a merge in progress. There is no abort command: the user
 * resolves (or aborts) in their git client, then retries.
 */
function ConflictBanner({
  files,
  retrying,
  onRetry,
}: {
  files: string[]
  retrying: boolean
  onRetry: () => void
}) {
  const { t } = useTranslation()
  const [showFiles, setShowFiles] = useState(false)

  return (
    <Alert variant="destructive" data-testid="remote-conflict-banner">
      <AlertTriangle />
      <AlertTitle>{t('remote.conflicts.title')}</AlertTitle>
      <AlertDescription>
        <p>{t('remote.conflicts.description', { count: files.length })}</p>
        <div className="mt-2 flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            aria-expanded={showFiles}
            onClick={() => setShowFiles(value => !value)}
          >
            {showFiles
              ? t('remote.conflicts.hideFiles')
              : t('remote.conflicts.showFiles')}
          </Button>
          <Tooltip>
            <TooltipTrigger asChild>
              {/* A disabled button gets no pointer events; the span does */}
              <span tabIndex={0}>
                <Button variant="outline" size="sm" disabled>
                  {t('remote.conflicts.abort')}
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>{t('remote.conflicts.abortHint')}</TooltipContent>
          </Tooltip>
          <Button
            variant="outline"
            size="sm"
            disabled={retrying}
            onClick={onRetry}
          >
            {retrying ? <Spinner /> : <RotateCw />}
            {t('remote.conflicts.retry')}
          </Button>
        </div>
        {showFiles && (
          <ul
            className="mt-2 list-inside list-disc font-mono text-xs"
            dir="ltr"
            aria-label={t('remote.conflicts.filesLabel')}
          >
            {files.map(file => (
              <li key={file}>{file}</li>
            ))}
          </ul>
        )}
      </AlertDescription>
    </Alert>
  )
}

/** The remote's branches; only the user's own can be pulled */
function BranchBrowser({
  ownBranch,
  pulling,
  onPull,
}: {
  ownBranch: string
  pulling: boolean
  onPull: () => void
}) {
  const { t } = useTranslation()
  const branches = useRemoteBranches(true)

  return (
    <section className="space-y-2" aria-labelledby="remote-branches-title">
      <h3 id="remote-branches-title" className="text-sm font-medium">
        {t('remote.branches.title')}
      </h3>
      {branches.isPending && <Skeleton className="h-16 w-full" />}
      {branches.isError && (
        <p role="alert" className="text-xs text-destructive">
          {t('remote.branches.loadFailed', {
            error: rawError(branches.error),
          })}
        </p>
      )}
      {branches.data && branches.data.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t('remote.branches.empty')}
        </p>
      )}
      {branches.data && branches.data.length > 0 && (
        <ul className="divide-y rounded-lg border">
          {branches.data.map(branch => {
            const own = branch === ownBranch
            return (
              <li
                key={branch}
                className={cn(
                  'flex items-center gap-2 px-3 py-2',
                  own && 'bg-accent/50'
                )}
                data-own={own || undefined}
              >
                <GitBranch className="size-4 shrink-0 text-muted-foreground" />
                <code
                  dir="ltr"
                  className="min-w-0 flex-1 truncate font-mono text-sm"
                >
                  {branch}
                </code>
                {own && (
                  <Badge variant="secondary">
                    {t('remote.branches.yours')}
                  </Badge>
                )}
                {own ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={pulling}
                    onClick={onPull}
                  >
                    {t('remote.branches.pull')}
                  </Button>
                ) : (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span tabIndex={0}>
                        <Button variant="ghost" size="sm" disabled>
                          {t('remote.branches.pull')}
                        </Button>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>
                      {t('remote.branches.onlyOwn')}
                    </TooltipContent>
                  </Tooltip>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}

/** Uncommitted files in the local repo (0 while unknown) */
function useLocalChanges(): number {
  const git = useQuery({
    queryKey: ['repo', 'git-status'],
    queryFn: async () => unwrapResult(await client.gitStatus()),
    retry: false,
  })
  return git.data?.files.length ?? 0
}
