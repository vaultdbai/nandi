import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight } from 'lucide-react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'

/**
 * One Settings section: a title, one plain sentence, actions, then the
 * content (at most 760 px wide). Technical material goes in `Advanced`.
 */
export function SettingsSectionFrame({
  title,
  description,
  actions,
  children,
}: {
  title: string
  description?: React.ReactNode
  actions?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <ScrollArea className="h-full min-h-0">
      <div className="flex w-full max-w-[760px] flex-col gap-6 p-6">
        <header className="flex flex-wrap items-start gap-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-lg font-semibold">{title}</h2>
            {description && (
              <p className="mt-1 text-sm text-muted-foreground">
                {description}
              </p>
            )}
          </div>
          {actions && (
            <div className="flex shrink-0 flex-wrap items-center gap-2">
              {actions}
            </div>
          )}
        </header>
        {children}
      </div>
    </ScrollArea>
  )
}

/** A labelled group inside a section (e.g. "Default model") */
export function SettingsGroup({
  title,
  description,
  children,
  className,
}: {
  title: string
  description?: React.ReactNode
  children: React.ReactNode
  className?: string
}) {
  return (
    <section className={cn('flex flex-col gap-3', className)}>
      <div>
        <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          {title}
        </h3>
        {description && (
          <p className="mt-1 text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      {children}
    </section>
  )
}

/** The collapsed "Advanced" (or "Details") block at the end of a section */
export function AdvancedBlock({
  label,
  children,
  defaultOpen = false,
}: {
  /** Defaults to "Advanced" */
  label?: string
  children: React.ReactNode
  defaultOpen?: boolean
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(defaultOpen)
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground">
        <ChevronRight
          className={cn(
            'size-4 transition-transform rtl:rotate-180',
            open && 'rotate-90 rtl:rotate-90'
          )}
        />
        {label ?? t('settings.advanced')}
      </CollapsibleTrigger>
      <CollapsibleContent className="mt-3 flex flex-col gap-4 ps-5">
        {children}
      </CollapsibleContent>
    </Collapsible>
  )
}
