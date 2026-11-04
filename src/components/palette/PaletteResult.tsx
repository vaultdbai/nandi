import {
  Bot,
  History,
  Package,
  Sparkles,
  Workflow,
  Wrench,
  Zap,
  type LucideIcon,
} from 'lucide-react'
import { Kbd } from '@/components/ui/kbd'
import { cn } from '@/lib/utils'
import type { PaletteItem, PaletteKind } from './palette-items'

const KIND_ICONS: Record<PaletteKind, LucideIcon> = {
  ask: Sparkles,
  action: Zap,
  agent: Bot,
  workflow: Workflow,
  run: History,
  package: Package,
  tool: Wrench,
}

interface PaletteResultProps {
  item: PaletteItem
  /** DOM id, for the input's `aria-activedescendant` */
  id: string
  active: boolean
  onSelect: () => void
  onHover: () => void
}

/** One palette result row: icon, name, description, shortcut */
export function PaletteResult({
  item,
  id,
  active,
  onSelect,
  onHover,
}: PaletteResultProps) {
  const Icon = KIND_ICONS[item.kind]

  return (
    <div
      id={id}
      role="option"
      aria-selected={active}
      data-active={active || undefined}
      onClick={onSelect}
      onMouseMove={onHover}
      className={cn(
        'flex cursor-default items-center gap-3 rounded-md px-3 py-2 text-sm select-none',
        active && 'bg-accent text-accent-foreground'
      )}
    >
      <Icon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{item.name}</span>
        {item.description && (
          <span className="truncate text-xs text-muted-foreground">
            {item.description}
          </span>
        )}
      </span>
      {item.shortcut && <Kbd className="shrink-0">{item.shortcut}</Kbd>}
    </div>
  )
}
