import { useTranslation } from 'react-i18next'
import { CircleCheck, Download, Hammer, TriangleAlert } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useBuildImage, useImageStatus, usePullImage } from '@/hooks/useImages'
import type { SandboxImage } from '@/lib/bindings'
import { cn } from '@/lib/utils'

/**
 * One sandbox image: here or not (and which version built it), Download
 * (the default: the image pinned to this app, from GitHub's container
 * registry) or Build here (the guided local build, for a computer that
 * can't reach the registry), and the output as it runs. `harness` is what
 * Claude Code, Codex and Gemini CLI run in; `runtime` is the tool sandbox
 * of Nandi's own agents. Says plainly when Docker isn't running.
 */
export function ImageBuildPanel({ image }: { image: SandboxImage }) {
  const { t } = useTranslation()
  const { data: status, isPending } = useImageStatus(image)
  const build = useBuildImage(image)
  const pull = usePullImage(image)
  const building = status?.building === true || build.isPending
  const pulling = status?.pulling === true || pull.isPending
  const busy = building || pulling
  const ready = status?.present === true && status.current
  const outdated = status?.present === true && !status.current
  const cantStart = isPending || busy || status?.dockerRunning === false
  const onError = { onError: (error: Error) => toast.error(error.message) }

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
        {!ready && !building && (
          <Button
            size="sm"
            disabled={cantStart}
            onClick={() => pull.mutate(undefined, onError)}
          >
            {pulling ? <Spinner /> : <Download />}
            {pulling ? t('images.downloading') : t('images.download')}
          </Button>
        )}
        {!pulling && (
          <Button
            size="sm"
            variant="outline"
            disabled={cantStart}
            onClick={() => build.mutate(undefined, onError)}
          >
            {building ? <Spinner /> : <Hammer />}
            {building
              ? t('images.building')
              : ready
                ? t('images.rebuild')
                : t('images.buildHere')}
          </Button>
        )}
      </div>
      {status && (
        <p
          className={cn(
            'flex items-center gap-1.5 text-xs',
            !status.dockerRunning || status.error || outdated
              ? 'text-warning'
              : 'text-muted-foreground'
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
                ? t('images.builtBy', {
                    tag: status.tag,
                    version: status.builtBy,
                  })
                : t('images.built', { tag: status.tag })}
            </>
          ) : outdated && !busy ? (
            <>
              <TriangleAlert className="size-3.5" />
              {t('images.outdated', { tag: status.tag })}
            </>
          ) : pulling ? (
            t('images.downloadingHint')
          ) : building ? (
            t('images.buildingHint')
          ) : (
            t('images.missing', { tag: status.tag })
          )}
        </p>
      )}
      {status && status.log.length > 0 && (
        <details open={busy} className="text-xs">
          <summary className="cursor-pointer text-muted-foreground">
            {t('images.log', { count: status.log.length })}
          </summary>
          <pre
            dir="ltr"
            className="mt-1 max-h-48 overflow-auto rounded bg-muted p-2 font-mono text-[11px] leading-snug whitespace-pre-wrap"
          >
            {status.log.join('\n')}
          </pre>
        </details>
      )}
    </div>
  )
}
