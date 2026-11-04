import { useTranslation } from 'react-i18next'
import { Info } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import type { ArtifactRef, EditPreview } from '@/lib/bindings'
import { cn } from '@/lib/utils'

/** A unified diff, colored by line */
export function DiffView({ diff }: { diff: string }) {
  const { t } = useTranslation()
  const lines = diff
    .split('\n')
    // The file header repeats what the card already says
    .filter(line => !/^(diff --git|index |--- |\+\+\+ )/.test(line))

  return (
    <pre
      aria-label={t('ask.edit.diff')}
      className="max-h-72 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-xs leading-relaxed"
    >
      {lines.map((line, index) => (
        <div
          key={index}
          className={cn(
            'px-1 whitespace-pre-wrap',
            line.startsWith('+') && 'bg-success/15 text-success',
            line.startsWith('-') && 'bg-destructive/15 text-destructive',
            line.startsWith('@@') && 'text-muted-foreground'
          )}
        >
          {line || ' '}
        </div>
      ))}
    </pre>
  )
}

/** "Agent writer", "Capability code-review.python", "NANDI.md" */
export function ArtifactLabel({ artifact }: { artifact: ArtifactRef }) {
  const { t } = useTranslation()
  return artifact.kind === 'gateway'
    ? t('ask.artifact.gateway')
    : t(`ask.artifact.${artifact.kind}`, { id: artifact.id })
}

/**
 * An edit preview card: what would change (diff), what it affects, the
 * version bump, and Apply / Dismiss. Nothing is written until applied.
 */
export function EditPreviewCard({
  preview,
  busy = false,
  onApply,
  onDismiss,
}: {
  preview: EditPreview
  busy?: boolean
  onApply?: () => void
  onDismiss?: () => void
}) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
        <ArtifactLabel artifact={preview.artifact} />
        {preview.versionBump && (
          <Badge variant="secondary">
            {t('ask.edit.bump', { bump: preview.versionBump })}
          </Badge>
        )}
      </div>
      <DiffView diff={preview.diff} />
      {preview.impact.length > 0 && (
        <ul aria-label={t('ask.edit.impact')} className="flex flex-col gap-1">
          {preview.impact.map(line => (
            <li
              key={line}
              className="flex items-start gap-2 text-xs text-muted-foreground"
            >
              <Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
              {line}
            </li>
          ))}
        </ul>
      )}
      {(onApply ?? onDismiss) && (
        <div className="flex justify-end gap-2">
          {onDismiss && (
            <Button size="sm" variant="ghost" onClick={onDismiss}>
              {t('ask.dismiss')}
            </Button>
          )}
          {onApply && (
            <Button size="sm" onClick={onApply} disabled={busy}>
              {t('ask.edit.apply')}
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
