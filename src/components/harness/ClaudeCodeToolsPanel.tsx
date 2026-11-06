import { useTranslation } from 'react-i18next'
import { CircleCheck, Download, TriangleAlert } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useInstallTools, useToolsStatus } from '@/hooks/useImages'
import { cn } from '@/lib/utils'
import { JobLog } from './JobLog'
import { formatDownload } from './image-progress'

/**
 * Claude Code in the sandbox: Nandi doesn't ship it (Anthropic's licence),
 * so it's fetched from npm on this computer the first time an agent needs
 * it, into a Docker volume its runs mount read-only. "Get it now" does that
 * ahead of time; after a failure the same button tries again.
 */
export function ClaudeCodeToolsPanel() {
  const { t, i18n } = useTranslation()
  const { data: status, isPending } = useToolsStatus('claudeCode')
  const install = useInstallTools('claudeCode')
  const installing = status?.installing === true || install.isPending
  const size = formatDownload(status?.downloadBytes, i18n.language) ?? ''
  const failed = Boolean(status?.error) && !installing

  return (
    <div
      className="flex flex-col gap-2 rounded-lg border p-3"
      data-testid="tools-claude-code"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium">
            {t('images.tools.claudeCode.title')}
          </span>
          <span className="block text-xs text-muted-foreground">
            {t('images.tools.claudeCode.description')}
          </span>
        </span>
        {status && !status.ready && (
          <Button
            size="sm"
            variant="outline"
            disabled={isPending || installing || !status.dockerRunning}
            onClick={() =>
              install.mutate(undefined, {
                onError: error => toast.error(error.message),
              })
            }
          >
            {installing ? <Spinner /> : <Download />}
            {installing
              ? t('images.tools.getting')
              : failed
                ? t('images.tryAgain')
                : t('images.tools.getNow')}
          </Button>
        )}
      </div>
      {status && (
        <p
          data-testid="tools-state"
          className={cn(
            'flex items-center gap-1.5 text-xs',
            failed ? 'text-warning' : 'text-muted-foreground'
          )}
        >
          {status.ready ? (
            <>
              <CircleCheck className="size-3.5 text-success" />
              {t('images.tools.ready')}
            </>
          ) : installing ? (
            t('images.tools.gettingHint', { size })
          ) : failed ? (
            <>
              <TriangleAlert className="size-3.5" />
              {t('images.failed', { error: status.error })}
            </>
          ) : (
            t('images.tools.onFirstUse', { size })
          )}
        </p>
      )}
      {status && <JobLog log={status.log} open={installing} />}
    </div>
  )
}
