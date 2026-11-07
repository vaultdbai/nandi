import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { client } from '@/lib/client'
import type { PullProgress } from '@/lib/bindings'
import { formatDownload, progressPercent } from './image-progress'

/**
 * Calm progress for what Nandi fetches by itself the first time an agent
 * needs it: a sandbox image's download (`images://pull`) and Claude Code's
 * one-time install (`images://tools`). One toast per job, updated in
 * place, ending in "ready" or a plain error with Try again. Mounted once,
 * so it shows whichever view is open.
 */
export function SandboxProgressToasts() {
  const { t, i18n } = useTranslation()

  useEffect(() => {
    const progressText = (
      title: string,
      progress: PullProgress | null | undefined
    ) => {
      const percent = progressPercent(progress)
      const total = formatDownload(progress?.totalBytes, i18n.language)
      return percent !== null && total
        ? t('images.toast.downloadingProgress', { title, total, percent })
        : t('images.toast.downloading', { title })
    }
    const progress = new Map<string, PullProgress>()
    const subscription = client.subscribe(event => {
      // The advanced build reports in Settings only
      if (event.event === 'images://pull' && !event.payload.build) {
        const { image, line, finished, error } = event.payload
        const id = `image-${image}`
        const title = t(`images.${image}.title`)
        if (event.payload.progress) progress.set(id, event.payload.progress)
        if (finished === true) {
          progress.delete(id)
          toast.success(t('images.toast.downloaded', { title }), { id })
        } else if (finished === false) {
          progress.delete(id)
          toast.error(t('images.toast.downloadFailed', { title }), {
            id,
            description: error,
            duration: Infinity,
            action: {
              label: t('images.tryAgain'),
              onClick: () => void client.pullImage(image),
            },
          })
        } else if (line !== null || event.payload.progress) {
          toast.loading(progressText(title, progress.get(id)), { id })
        }
      } else if (event.event === 'images://tools') {
        const { bundle, downloadBytes, finished, error } = event.payload
        const id = `tools-${bundle}`
        if (finished === true) {
          toast.success(t('images.tools.toast.ready'), { id })
        } else if (finished === false) {
          toast.error(t('images.tools.toast.failed'), {
            id,
            description: error,
            duration: Infinity,
            action: {
              label: t('images.tryAgain'),
              onClick: () => void client.installTools(bundle),
            },
          })
        } else {
          const size = formatDownload(downloadBytes, i18n.language) ?? ''
          toast.loading(t('images.tools.toast.getting', { size }), { id })
        }
      }
    })
    return () => subscription.unsubscribe()
  }, [t, i18n.language])

  return null
}
