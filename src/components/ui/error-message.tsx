import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight } from 'lucide-react'
import {
  friendlyErrorKey,
  friendlyErrorParams,
  rawError,
} from '@/lib/friendly-error'
import { cn } from '@/lib/utils'

/**
 * A backend error in plain words, with the raw text behind "Details".
 * `message` replaces the generic sentence when the screen knows better.
 */
export function ErrorMessage({
  error,
  message,
  className,
  detailsOpen = false,
}: {
  error: unknown
  message?: string
  className?: string
  /** Start with Details shown (e.g. a file editor, where the raw text
   * says which line) */
  detailsOpen?: boolean
}) {
  const { t, i18n } = useTranslation()
  const [open, setOpen] = useState(detailsOpen)
  const raw = rawError(error)
  return (
    <div role="alert" className={cn('text-sm text-destructive', className)}>
      <p>
        {message ??
          t(friendlyErrorKey(error), friendlyErrorParams(error, i18n.language))}
      </p>
      {raw && (
        <>
          <button
            type="button"
            aria-expanded={open}
            className="mt-1 flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setOpen(value => !value)}
          >
            <ChevronRight
              aria-hidden
              className={cn(
                'size-3 transition-transform rtl:rotate-180',
                open && 'rotate-90 rtl:rotate-90'
              )}
            />
            {t('errors.details')}
          </button>
          {open && (
            <pre
              dir="ltr"
              className="mt-1 max-h-40 overflow-auto rounded-md bg-muted p-2 text-start font-mono text-xs whitespace-pre-wrap text-muted-foreground"
            >
              {raw}
            </pre>
          )}
        </>
      )}
    </div>
  )
}
