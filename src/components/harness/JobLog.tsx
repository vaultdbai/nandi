import { useTranslation } from 'react-i18next'

/** The last lines of a download or install, open while it runs */
export function JobLog({ log, open }: { log: string[]; open: boolean }) {
  const { t } = useTranslation()
  if (log.length === 0) return null
  return (
    <details open={open} className="text-xs">
      <summary className="cursor-pointer text-muted-foreground">
        {t('images.log', { count: log.length })}
      </summary>
      <pre
        dir="ltr"
        className="mt-1 max-h-48 overflow-auto rounded bg-muted p-2 font-mono text-[11px] leading-snug whitespace-pre-wrap"
      >
        {log.join('\n')}
      </pre>
    </details>
  )
}
