import moving from '@/assets/brand/mascot/companion-bison.webp'
import still from '@/assets/brand/mascot/companion-bison-still.webp'
import { cn } from '@/lib/utils'
import './companion-bison.css'

/**
 * The companion's bison, lying down and breathing in a 4-second loop
 * (a brand asset, see NOTICE). It holds still when Settings ›
 * Appearance › "Bison moves" says so (`companion-bison.css`). Decorative:
 * the caller labels it.
 */
export function CompanionBison({ className }: { className?: string }) {
  return (
    <span aria-hidden className={cn('pointer-events-none', className)}>
      <img
        className="companion-bison-moving size-full object-contain"
        src={moving}
        alt=""
        draggable={false}
      />
      <img
        className="companion-bison-still size-full object-contain"
        src={still}
        alt=""
        draggable={false}
      />
    </span>
  )
}
