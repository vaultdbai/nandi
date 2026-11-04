import { cn } from '@/lib/utils'

interface ViewHeaderProps {
  title: string
  /** Inline after the title, e.g. a status badge */
  titleAdornment?: React.ReactNode
  subtitle?: React.ReactNode
  /** Leading element, e.g. an avatar or back button */
  leading?: React.ReactNode
  /** Trailing actions */
  actions?: React.ReactNode
  className?: string
}

/** The fixed 56px header every main-pane view starts with */
export function ViewHeader({
  title,
  titleAdornment,
  subtitle,
  leading,
  actions,
  className,
}: ViewHeaderProps) {
  return (
    <header
      className={cn(
        'flex h-14 shrink-0 items-center gap-3 border-b px-4',
        className
      )}
    >
      {leading}
      <div className="min-w-0 flex-1">
        {titleAdornment ? (
          <div className="flex min-w-0 items-center gap-2">
            <h1 className="truncate text-sm font-semibold text-foreground">
              {title}
            </h1>
            {titleAdornment}
          </div>
        ) : (
          <h1 className="truncate text-sm font-semibold text-foreground">
            {title}
          </h1>
        )}
        {subtitle && (
          <div className="truncate text-xs text-muted-foreground">
            {subtitle}
          </div>
        )}
      </div>
      {actions && (
        <div className="flex shrink-0 items-center gap-2">{actions}</div>
      )}
    </header>
  )
}
