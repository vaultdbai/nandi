import { useTranslation } from 'react-i18next'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { HarnessInfo, HarnessTestResult } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

/** A harness's detection details and its last Test connection result */
export function HarnessHealthDialog({
  harness,
  test,
  onOpenChange,
}: {
  harness: HarnessInfo | null
  test?: HarnessTestResult | null
  onOpenChange: (open: boolean) => void
}) {
  const { t, i18n } = useTranslation()
  const health = harness?.health
  const rows: [string, string][] = harness
    ? [
        [t('harness.health.executable'), harness.executable ?? '—'],
        [t('harness.health.version'), health?.version ?? '—'],
        [t('harness.health.configDir'), harness.configDir ?? '—'],
        [t('harness.health.provider'), health?.configuredProvider ?? '—'],
        [
          t('harness.health.signedIn'),
          health?.hostAuthenticated ? t('harness.yes') : t('harness.no'),
        ],
        [
          t('harness.health.containerSignedIn'),
          health?.containerAuthenticated
            ? t('harness.yes')
            : t('harness.health.containerNeedsKey'),
        ],
        [
          t('harness.health.container'),
          harness.imageAvailable ? t('harness.yes') : t('harness.no'),
        ],
        [
          t('harness.health.checked'),
          health?.lastCheck
            ? formatRelativeTime(health.lastCheck, i18n.language)
            : '—',
        ],
      ]
    : []

  return (
    <Dialog open={harness !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{harness?.displayName}</DialogTitle>
          <DialogDescription>
            {t('harness.health.description')}
          </DialogDescription>
        </DialogHeader>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          {rows.map(([term, value]) => (
            <div key={term} className="contents">
              <dt className="text-muted-foreground">{term}</dt>
              <dd className="min-w-0 truncate font-mono text-xs leading-5">
                {value}
              </dd>
            </div>
          ))}
        </dl>
        {health?.error && (
          <p className="text-sm text-destructive">{health.error}</p>
        )}
        {test && (
          <div className="rounded-md border p-3 text-sm">
            <p className={test.ok ? '' : 'text-destructive'}>{test.message}</p>
            {test.error && (
              <p className="mt-1 font-mono text-xs break-all text-muted-foreground">
                {test.error}
              </p>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
