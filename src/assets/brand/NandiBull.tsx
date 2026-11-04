import { useTranslation } from 'react-i18next'
import { useTheme } from '@/hooks/use-theme'
import { cn } from '@/lib/utils'
import { PALETTE_ICONS } from '@/themes/palette-icons'
import { BisonPose } from './BisonPose'
import './nandi-bull.css'

/**
 * The mascot (a brand asset, see NOTICE). From 48 px up it's the
 * whole bison in a pose per state: `rest` (nothing needs you) lies down,
 * `wait` (an approval is pending) stands with an amber badge, `carry`
 * (agents are running) carries crates, `done` (just approved) shows a
 * check, and each moves like a bison (`BisonPose`). Smaller, it's the
 * palette's app icon with the badge on top. Motion stops under reduced
 * motion.
 */
export type NandiState = 'rest' | 'wait' | 'carry' | 'done'

export function NandiBull({
  state = 'rest',
  size = 160,
  name = 'Nandi',
  label,
  className,
}: {
  state?: NandiState
  size?: number | string
  /** The bull's name, for its spoken label */
  name?: string
  label?: string
  className?: string
}) {
  const { t } = useTranslation()
  const { palette } = useTheme()
  // A whole bison is unreadable at title-bar size
  const pose = typeof size !== 'number' || size >= 48
  return (
    <span
      className={cn('nandi-bull', className)}
      data-state={state}
      data-pose={pose || undefined}
      style={{ width: size, height: size }}
      role="img"
      aria-label={label ?? t(`mascot.state.${state}`, { name })}
    >
      {pose ? (
        <BisonPose pose={state} className="size-full" />
      ) : (
        <img
          className="nb-icon"
          src={PALETTE_ICONS[palette]}
          alt=""
          draggable={false}
        />
      )}
      {!pose && (
        <>
          <span className="nb-alert" />
          <svg className="nb-check" viewBox="0 0 16 16" aria-hidden="true">
            <circle cx="8" cy="8" r="8" />
            <path d="M4.5 8.2 l2.4 2.4 l4.6 -4.8" />
          </svg>
        </>
      )}
    </span>
  )
}
