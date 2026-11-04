import { useState } from 'react'
import { Bot, Package, Plug, Workflow } from 'lucide-react'
import { cn } from '@/lib/utils'

interface MarketplaceIconProps {
  /** `https://…` from the registry entry */
  url: string | null
  /** Picks the fallback glyph (`tool`: connectors and pilots) */
  kind: 'pack' | 'agent' | 'workflow' | 'tool'
  className?: string
}

/** Category-coloured tile per kind (see `--kind-*` in theme-variables.css) */
const TILES = {
  pack: 'border-kind-pack/25 bg-kind-pack/12 text-kind-pack',
  agent: 'border-kind-agent/25 bg-kind-agent/12 text-kind-agent',
  workflow: 'border-kind-workflow/25 bg-kind-workflow/12 text-kind-workflow',
  tool: 'border-kind-tool/25 bg-kind-tool/12 text-kind-tool',
}

const FALLBACKS = {
  pack: Package,
  agent: Bot,
  workflow: Workflow,
  tool: Plug,
}

/**
 * A pack's or agent's icon. Registry icons are monochrome SVGs drawn in
 * black, so they're inverted in dark mode. Falls back to a generic glyph if
 * there's no icon or it doesn't load (offline, private registry). The
 * webview's HTTP cache keeps them after the first load.
 */
export function MarketplaceIcon({
  url,
  kind,
  className,
}: MarketplaceIconProps) {
  const [failed, setFailed] = useState(false)
  const Fallback = FALLBACKS[kind]
  const box = cn(
    'flex size-10 shrink-0 items-center justify-center rounded-lg border',
    TILES[kind],
    className
  )

  if (!url?.startsWith('https://') || failed) {
    return (
      <div className={box} aria-hidden>
        <Fallback className="size-5" />
      </div>
    )
  }
  return (
    <div className={box} aria-hidden>
      <img
        src={url}
        alt=""
        className="size-6 dark:invert"
        loading="lazy"
        referrerPolicy="no-referrer"
        onError={() => setFailed(true)}
      />
    </div>
  )
}
