import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { CompanionBison } from '@/assets/brand/CompanionBison'
import type { NandiState } from '@/assets/brand/NandiBull'
import type { Orb, OrbState } from './companion-model'

/** The count's colour: what it counts */
const BADGE_COLORS: Record<OrbState, string> = {
  needsYou: 'bg-warning text-black/80',
  problem: 'bg-destructive text-white',
  running: 'bg-info text-white',
  quiet: 'bg-foreground text-background',
}

/** Moving this far (CSS pixels) with the button down is a drag, not a click */
const DRAG_THRESHOLD = 5

/**
 * The collapsed companion: the bison on its own, lying down and breathing
 * (`CompanionBison`, white with a dark outline and a soft shadow, so it
 * reads on any wallpaper), with the Inbox count (the Home badge's) or `!`
 * for a problem. `pose` is exposed as `data-pose` for styling and tests. A
 * click opens the card; dragging moves the window (`onDragStart`), which
 * snaps to a screen corner when dropped.
 */
export function CompanionCollapsed({
  orb,
  pose,
  tooltip,
  onExpand,
  onDragStart,
  onMenu,
}: {
  orb: Orb
  pose: NandiState
  /** "2 need you · 3 running", or the problem's sentence */
  tooltip: string
  onExpand: () => void
  /** Starts moving the window; without it the bison can't be dragged */
  onDragStart?: () => void
  /** Right-click: the bison's menu (desktop) */
  onMenu?: () => void
}) {
  const { t } = useTranslation()
  const down = useRef<{ x: number; y: number } | null>(null)
  // The OS drag can swallow the pointerup, so the next press resets it
  const dragged = useRef(false)

  return (
    <button
      type="button"
      data-state={orb.state}
      data-pose={pose}
      aria-label={t('companion.orb', { status: tooltip })}
      title={tooltip}
      onPointerDown={event => {
        if (event.button !== 0) return
        down.current = { x: event.clientX, y: event.clientY }
        dragged.current = false
      }}
      onPointerMove={event => {
        const start = down.current
        if (!start || dragged.current || !onDragStart) return
        const moved = Math.hypot(
          event.clientX - start.x,
          event.clientY - start.y
        )
        if (moved > DRAG_THRESHOLD) {
          dragged.current = true
          down.current = null
          onDragStart()
        }
      }}
      onPointerUp={() => {
        down.current = null
      }}
      onContextMenu={event => {
        if (!onMenu) return
        event.preventDefault()
        onMenu()
      }}
      onClick={() => {
        if (dragged.current) {
          dragged.current = false
          return
        }
        onExpand()
      }}
      className={cn(
        'relative flex h-28 w-44 cursor-grab touch-none items-center justify-center rounded-2xl transition-transform outline-none select-none hover:scale-105 focus-visible:ring-[3px] focus-visible:ring-ring/50 active:cursor-grabbing'
      )}
    >
      <CompanionBison className="flex h-28 w-44 items-center drop-shadow-[0_3px_6px_rgb(0_0_0/0.45)]" />
      {orb.badge !== null && (
        <span
          aria-hidden
          data-testid="orb-badge"
          className={cn(
            'absolute end-1 top-1 flex min-w-7 items-center justify-center rounded-full border-2 border-background px-1.5 py-0.5 text-sm font-semibold shadow',
            BADGE_COLORS[orb.state]
          )}
        >
          {orb.badge}
        </span>
      )}
    </button>
  )
}
