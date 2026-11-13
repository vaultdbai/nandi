import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import { FolderGit2, GitBranch } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { client } from '@/lib/client'
import { rawError } from '@/lib/friendly-error'
import { unwrapResult } from '@/lib/tauri-bindings'

const repoStatusQueryKey = ['repo', 'status'] as const
const gitStatusQueryKey = ['repo', 'git-status'] as const

/** Settings › Backup › This computer's copy: the local repo and its state */
export function LocalCopyPanel() {
  const { t } = useTranslation()
  const status = useQuery({
    queryKey: repoStatusQueryKey,
    queryFn: async () => unwrapResult(await client.repoStatus()),
  })
  // Git is optional for the panel: a repo without commits still shows
  const git = useQuery({
    queryKey: gitStatusQueryKey,
    queryFn: async () => unwrapResult(await client.repoGitStatus()),
    retry: false,
  })

  if (status.isPending) {
    return (
      <div className="flex items-center gap-2 p-6 text-sm text-muted-foreground">
        <Spinner />
        {t('repoSetup.view.loading')}
      </div>
    )
  }
  if (status.isError) {
    return (
      <div className="flex flex-col items-start gap-2 p-2 text-sm">
        <p role="alert" className="text-destructive">
          {rawError(status.error)}
        </p>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void status.refetch()}
        >
          {t('repoSetup.tryAgain')}
        </Button>
      </div>
    )
  }

  const repo = status.data
  const errors = repo.problems.filter(problem => problem.isError).length
  return (
    <div className="flex w-full flex-col gap-6">
      <section
        className="space-y-3 rounded-lg border p-4"
        data-testid="repo-setup-current"
      >
        <div className="flex items-center gap-2">
          <FolderGit2 className="size-5 text-muted-foreground" />
          <h3 className="min-w-0 flex-1 truncate font-medium">
            {t('repoSetup.view.localRepo')}
          </h3>
          <Badge variant="secondary">
            {repo.watching
              ? t('repoSetup.view.watching')
              : t('repoSetup.view.notWatching')}
          </Badge>
        </div>
        <p
          className="break-all font-mono text-sm text-muted-foreground"
          dir="ltr"
        >
          {repo.root}
        </p>
        <p className="text-sm">
          {t('repoSetup.view.counts', {
            agents: repo.agents,
            workflows: repo.workflows,
            triggers: repo.triggers,
          })}
        </p>
        {repo.problems.length > 0 && (
          <p className="text-sm text-destructive">
            {t('repoSetup.view.problems', {
              count: repo.problems.length,
              errors,
            })}
          </p>
        )}
        {git.data && (
          <p
            className="flex items-center gap-1.5 text-sm text-muted-foreground"
            data-testid="repo-setup-git"
          >
            <GitBranch className="size-4" />
            {t('repoSetup.view.git', {
              branch: git.data.branch,
              changes: git.data.files.length,
            })}
          </p>
        )}
      </section>
    </div>
  )
}
