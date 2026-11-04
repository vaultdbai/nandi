import { useTranslation } from 'react-i18next'
import { CheckCircle2, XCircle } from 'lucide-react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import type { LlmKeyTest } from '@/lib/bindings'
import { keyTestOutcome } from '@/lib/models'

/**
 * One line saying whether a key works, in plain words; the provider's raw
 * answer stays behind Details.
 */
export function KeyTestResult({ test }: { test: LlmKeyTest }) {
  const { t } = useTranslation()
  const outcome = keyTestOutcome(test)

  if (outcome === 'works') {
    return (
      <p
        role="status"
        className="flex items-center gap-1.5 text-sm text-success"
      >
        <CheckCircle2 aria-hidden className="size-4 shrink-0" />
        {t('models.test.works', { ms: test.latencyMs })}
      </p>
    )
  }

  return (
    <div role="alert" className="space-y-1">
      <p className="flex items-center gap-1.5 text-sm text-destructive">
        <XCircle aria-hidden className="size-4 shrink-0" />
        {t(`models.test.${outcome}`)}
      </p>
      {test.detail && (
        <Collapsible>
          <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-2 hover:underline">
            {t('models.test.details')}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <pre
              dir="ltr"
              className="mt-1 max-h-32 overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap"
            >
              {test.detail}
            </pre>
          </CollapsibleContent>
        </Collapsible>
      )}
    </div>
  )
}
