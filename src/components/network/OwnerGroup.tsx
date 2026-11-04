import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronDown } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { cn } from '@/lib/utils'
import {
  OwnerGroupHeader,
  type OwnerGroupHeaderProps,
} from './OwnerGroupHeader'

export interface OwnerGroupProps extends Omit<
  OwnerGroupHeaderProps,
  'children'
> {
  /** Extra controls in the header (e.g. `TrustPopover`) */
  actions?: ReactNode
  /** The owner's agent cards */
  children: ReactNode
}

/**
 * One owner's agents in the shared grids: "Alice (3 agents)" with a
 * toggle that shows or hides the cards (open at first)
 */
export function OwnerGroup({ actions, children, ...header }: OwnerGroupProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(true)
  const name = header.displayName || header.handle || t('shared.unknownOwner')
  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className="flex flex-col gap-3 md:col-span-2 xl:col-span-3"
      asChild
    >
      <section>
        <div className="flex items-center gap-2">
          <CollapsibleTrigger asChild>
            <Button
              size="icon"
              variant="ghost"
              className="size-7 shrink-0"
              aria-label={t('shared.toggleGroup', { name })}
            >
              <ChevronDown
                className={cn('transition-transform', !open && '-rotate-90')}
              />
            </Button>
          </CollapsibleTrigger>
          <div className="min-w-0 flex-1">
            <OwnerGroupHeader {...header}>{actions}</OwnerGroupHeader>
          </div>
        </div>
        <CollapsibleContent>
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
            {children}
          </div>
        </CollapsibleContent>
      </section>
    </Collapsible>
  )
}
