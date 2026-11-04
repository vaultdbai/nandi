/**
 * A small single-series bar chart in plain SVG, for numbers a run's output
 * stated (see `detectNumericData`). One series: no legend, the title names
 * it; each bar has a hover tooltip and the values are also in the cards.
 */
import { useState } from 'react'
import type { NumericPoint } from './run-detail-model'

const WIDTH = 480
const HEIGHT = 180
const PAD = { top: 12, right: 8, bottom: 28, left: 8 }
const GAP = 2

export function RunChart({
  title,
  points,
}: {
  title: string
  points: NumericPoint[]
}) {
  const [hovered, setHovered] = useState<number | null>(null)
  const values = points.map(p => p.value)
  const max = Math.max(0, ...values)
  const min = Math.min(0, ...values)
  const span = max - min || 1
  const plotW = WIDTH - PAD.left - PAD.right
  const plotH = HEIGHT - PAD.top - PAD.bottom
  const band = plotW / points.length
  const barW = Math.max(2, Math.min(36, band - GAP))
  const y = (v: number) => PAD.top + ((max - v) / span) * plotH
  const baseline = y(0)
  // Label every bar when they fit, else roughly six
  const labelEvery = Math.max(1, Math.ceil(points.length / 6))
  const active = hovered === null ? null : points[hovered]

  return (
    <figure className="flex flex-col gap-1">
      <figcaption className="flex items-baseline justify-between gap-2 text-sm font-medium">
        <span>{title}</span>
        {active && (
          <span className="text-xs font-normal text-muted-foreground tabular-nums">
            {active.label}: {active.display}
          </span>
        )}
      </figcaption>
      <svg
        role="img"
        aria-label={title}
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="h-auto w-full"
        onMouseLeave={() => setHovered(null)}
      >
        <line
          x1={PAD.left}
          x2={WIDTH - PAD.right}
          y1={baseline}
          y2={baseline}
          className="stroke-border"
        />
        {points.map((point, i) => {
          const x = PAD.left + i * band + (band - barW) / 2
          const top = Math.min(y(point.value), baseline)
          const height = Math.max(1, Math.abs(y(point.value) - baseline))
          return (
            <g key={`${point.label}-${i}`}>
              {/* Hit target: the whole band, taller than the bar */}
              <rect
                x={PAD.left + i * band}
                y={PAD.top}
                width={band}
                height={plotH}
                fill="transparent"
                onMouseEnter={() => setHovered(i)}
              >
                <title>{`${point.label}: ${point.display}`}</title>
              </rect>
              <rect
                x={x}
                y={top}
                width={barW}
                height={height}
                rx={Math.min(4, barW / 2)}
                className={hovered === i ? 'fill-primary' : 'fill-primary/70'}
                pointerEvents="none"
              />
              {i % labelEvery === 0 && (
                <text
                  x={PAD.left + i * band + band / 2}
                  y={HEIGHT - 8}
                  textAnchor="middle"
                  className="fill-muted-foreground text-[10px]"
                >
                  {point.label.length > 10
                    ? `${point.label.slice(0, 9)}…`
                    : point.label}
                </text>
              )}
            </g>
          )
        })}
      </svg>
    </figure>
  )
}
