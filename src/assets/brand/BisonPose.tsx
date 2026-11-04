import { cn } from '@/lib/utils'
import { MASCOT_POSES } from '@/themes/palette-icons'
import type { NandiState } from './NandiBull'
import './bison-pose.css'

/** Which parts move in each pose (`bison-pose.css` has their shapes) */
const PARTS: Record<NandiState, readonly string[]> = {
  rest: ['head'],
  wait: ['head', 'tail', 'badge'],
  carry: ['head', 'tail', 'crates'],
  done: ['head', 'tail', 'badge'],
}

/**
 * The bison in `pose`, moving the way bison do: the pose image stacked as
 * its body plus a copy per moving part (`bison-pose.css` cuts and moves
 * them). At rest it lies breathing, dozes and looks up, and now and then
 * gets up for a stretch; waiting, it looks at you and swishes its tail;
 * carrying, it walks; done, the check pops in and it nods. Decorative:
 * the caller labels it. Still under reduced motion.
 */
export function BisonPose({
  pose,
  className,
}: {
  pose: NandiState
  className?: string
}) {
  const src = MASCOT_POSES[pose]
  return (
    <span aria-hidden className={cn('bison', className)} data-pose={pose}>
      <span className="bp-group">
        <img className="bp-part bp-base" src={src} alt="" draggable={false} />
        {PARTS[pose].map(part => (
          <img
            key={part}
            className={`bp-part bp-${part}`}
            src={src}
            alt=""
            draggable={false}
          />
        ))}
      </span>
      {pose === 'rest' && (
        // Getting up: the standing pose, without its badge
        <img
          className="bp-part bp-stand"
          src={MASCOT_POSES.wait}
          alt=""
          draggable={false}
        />
      )}
    </span>
  )
}
