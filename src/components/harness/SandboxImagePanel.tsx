import { useTranslation } from 'react-i18next'
import { CircleCheck, Download, Hammer, TriangleAlert } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useBuildImage, useImageStatus, usePullImage } from '@/hooks/useImages'
import type { ImageStatus, SandboxImage } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { JobLog } from './JobLog'
import { formatDownload, progressPercent } from './image-progress'

/**
 * One sandbox image: here or not, and Download (the image pinned to this
 * app, from GitHub's container registry) with its size and progress.
 * Nandi downloads it by itself the first time an agent needs it, so the
 * button is only for getting it ahead of time, or trying again. Says
 * plainly when Docker isn't running. `harness` is what Codex, Gemini CLI
 * and Claude Code run in; `runtime` is the tool sandbox of Nandi's own
 * agents. Building here is in Advanced (`SandboxBuildPanel`).
 */
export function SandboxImagePanel({ image }: { image: SandboxImage }) {
  const { t, i18n } = useTranslation()
  const { data: status, isPending } = useImageStatus(image)
  const pull = usePullImage(image)
  const pulling = status?.pulling === true || pull.isPending
  const busy = pulling || status?.building === true
  const ready = status?.present === true && status.current
  const size = formatDownload(status?.downloadBytes, i18n.language)
  const percent = progressPercent(status?.progress)

  return (
    <div
      className="flex flex-col gap-2 rounded-lg border p-3"
      data-testid={`image-${image}`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium">
            {t(`images.${image}.title`)}
          </span>
          <span className="block text-xs text-muted-foreground">
            {t(`images.${image}.description`)}
          </span>
        </span>
        {!ready && status?.building !== true && (
          <Button
            size="sm"
            disabled={isPending || busy || status?.dockerRunning === false}
            onClick={() =>
              pull.mutate(undefined, {
                onError: error => toast.error(error.message),
              })
            }
          >
            {pulling ? <Spinner /> : <Download />}
            {pulling
              ? t('images.downloading')
              : status?.error
                ? t('images.tryAgain')
                : size
                  ? t('images.downloadSize', { size })
                  : t('images.download')}
          </Button>
        )}
      </div>
      {pulling && (
        <div
          role="progressbar"
          aria-label={t('images.downloading')}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent ?? undefined}
          className="h-1.5 overflow-hidden rounded-full bg-muted"
        >
          <div
            className={cn(
              'h-full rounded-full bg-primary transition-[width]',
              percent === null && 'w-1/3 animate-pulse'
            )}
            style={percent === null ? undefined : { width: `${percent}%` }}
          />
        </div>
      )}
      {status && (
        <ImageState
          status={status}
          busy={busy}
          size={size}
          percent={percent}
          locale={i18n.language}
        />
      )}
      {status && <JobLog log={status.log} open={busy} />}
    </div>
  )
}

function ImageState({
  status,
  busy,
  size,
  percent,
  locale,
}: {
  status: ImageStatus
  busy: boolean
  size: string | null
  percent: number | null
  locale: string
}) {
  const { t } = useTranslation()
  const ready = status.present && status.current
  const warn = !status.dockerRunning || (status.error !== null && !busy)
  return (
    <p
      className={cn(
        'flex items-center gap-1.5 text-xs',
        warn ? 'text-warning' : 'text-muted-foreground'
      )}
      data-testid="image-state"
    >
      {!status.dockerRunning ? (
        <>
          <TriangleAlert className="size-3.5" />
          {t('images.dockerNotRunning')}
        </>
      ) : status.error && !busy ? (
        <>
          <TriangleAlert className="size-3.5" />
          {t('images.failed', { error: status.error })}
        </>
      ) : ready ? (
        <>
          <CircleCheck className="size-3.5 text-success" />
          {status.builtBy
            ? t('images.readyBuiltBy', {
                tag: status.tag,
                version: status.builtBy,
              })
            : t('images.ready', { tag: status.tag })}
        </>
      ) : status.pulling ? (
        percent !== null && status.progress?.totalBytes ? (
          t('images.downloadingProgress', {
            done: formatDownload(status.progress.doneBytes, locale) ?? '0',
            total: formatDownload(status.progress.totalBytes, locale),
            percent,
          })
        ) : (
          t('images.downloadingHint')
        )
      ) : status.building ? (
        t('images.buildingHint')
      ) : status.present ? (
        t('images.outdated', { tag: status.tag })
      ) : size ? (
        t('images.missingSize', { size })
      ) : (
        t('images.missing')
      )}
    </p>
  )
}

/**
 * Advanced, never required: build the sandbox images on this computer
 * instead of downloading them (the same name and labels, so the rest of
 * Nandi doesn't care which). Its output shows in the image's panel.
 */
export function SandboxBuildPanel() {
  const { t } = useTranslation()
  return (
    <div
      className="flex flex-col gap-2 rounded-lg border p-3"
      data-testid="sandbox-build"
    >
      <span className="block text-sm font-medium">
        {t('images.buildTitle')}
      </span>
      <span className="block text-xs text-muted-foreground">
        {t('images.buildDescription')}
      </span>
      <div className="flex flex-wrap gap-2">
        <BuildButton image="runtime" />
        <BuildButton image="harness" />
      </div>
    </div>
  )
}

function BuildButton({ image }: { image: SandboxImage }) {
  const { t } = useTranslation()
  const { data: status, isPending } = useImageStatus(image)
  const build = useBuildImage(image)
  const building = status?.building === true || build.isPending
  return (
    <Button
      size="sm"
      variant="outline"
      disabled={
        isPending ||
        building ||
        status?.pulling === true ||
        status?.dockerRunning === false
      }
      onClick={() =>
        build.mutate(undefined, {
          onError: error => toast.error(error.message),
        })
      }
    >
      {building ? <Spinner /> : <Hammer />}
      {building
        ? t('images.building')
        : t('images.buildHere', { name: t(`images.${image}.title`) })}
    </Button>
  )
}
