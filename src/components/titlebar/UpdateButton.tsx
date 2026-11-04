import { useTranslation } from 'react-i18next'
import { Download, Loader2, RotateCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { installUpdate, useUpdateStore } from '@/store/update-store'

/**
 * Shown in the title bar once a newer version is available. One click
 * downloads it, installs it, and restarts the app.
 */
export function UpdateButton() {
  const { t } = useTranslation()
  const status = useUpdateStore(state => state.status)
  const version = useUpdateStore(state => state.version)
  const progress = useUpdateStore(state => state.progress)
  const error = useUpdateStore(state => state.error)

  if (!version) return null
  const busy = status === 'downloading' || status === 'installing'
  const failed = status === 'error'

  let label = t('update.button', { version })
  if (status === 'downloading') {
    label =
      progress === null
        ? t('update.downloading')
        : t('update.downloadingPercent', { progress })
  } else if (status === 'installing') {
    label = t('update.installing')
  } else if (failed) {
    label = t('update.retry')
  }

  return (
    <Button
      onClick={() => void installUpdate()}
      disabled={busy}
      size="sm"
      variant={failed ? 'destructive' : 'default'}
      className="h-7 min-w-7 gap-1 px-2 text-xs"
      title={failed && error ? error : t('update.tooltip', { version })}
    >
      {busy ? (
        <Loader2 className="h-3 w-3 animate-spin" />
      ) : failed ? (
        <RotateCw className="h-3 w-3" />
      ) : (
        <Download className="h-3 w-3" />
      )}
      {label}
    </Button>
  )
}
