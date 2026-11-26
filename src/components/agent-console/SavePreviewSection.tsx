import { useTranslation } from 'react-i18next'
import { useSavePreview } from '@/hooks/useSavePreview'
import type { JsonValue } from '@/lib/bindings'

/**
 * What a save creates and may reach, read by the core from the call's own
 * file (`preview_save`): its name, whether it replaces one, its sites,
 * connectors and tools, what it runs, and why it would be refused
 */
export function SavePreviewSection({
  approvalId,
  args,
}: {
  approvalId: string
  args: JsonValue
}) {
  const { t } = useTranslation()
  const { data: preview } = useSavePreview(approvalId, args)
  if (!preview) return null
  const kind = t(`savePreview.kind.${preview.kind}`, {
    defaultValue: preview.kind,
  })
  const name = preview.name || preview.id
  const rows: [string, string[]][] = [
    ['savePreview.sites', preview.sites],
    ['savePreview.connectors', preview.connectors],
    ['savePreview.tools', preview.tools],
    ['savePreview.runs', preview.runs],
  ]

  return (
    <section
      aria-label={t('savePreview.label')}
      className="space-y-2 rounded-md border bg-card px-3 py-2 text-sm"
    >
      <p className="font-medium">
        {t(preview.replaces ? 'savePreview.replaces' : 'savePreview.creates', {
          kind,
          name,
        })}
      </p>
      {preview.kind === 'agent' &&
        preview.sites.length === 0 &&
        preview.connectors.length === 0 && (
          <p className="text-xs text-muted-foreground">
            {t('savePreview.reachesNothing')}
          </p>
        )}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        {rows
          .filter(([, values]) => values.length > 0)
          .map(([label, values]) => (
            <div key={label} className="contents">
              <dt className="text-muted-foreground">{t(label)}</dt>
              <dd className="break-words">{values.join(', ')}</dd>
            </div>
          ))}
      </dl>
      {preview.problems.length > 0 && (
        <ul role="alert" className="list-disc ps-4 text-xs text-destructive">
          {preview.problems.map(problem => (
            <li key={problem}>{problem}</li>
          ))}
        </ul>
      )}
    </section>
  )
}
