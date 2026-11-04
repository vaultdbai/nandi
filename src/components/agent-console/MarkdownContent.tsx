import { useContext, type ReactNode } from 'react'
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
} from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { openUrl } from '@tauri-apps/plugin-opener'
import { isDesktop } from '@/lib/client'
import { logger } from '@/lib/logger'
import { MarkdownLinkContext } from './markdown-link-context'

/** In-app links the default transform would blank (`nandi://request/<id>`) */
const IN_APP_LINK = /^nandi:\/\/request\//

function MarkdownLink({
  href,
  children,
}: {
  href: string | undefined
  children: ReactNode
}) {
  const render = useContext(MarkdownLinkContext)
  const custom = href && render ? render(href, children) : null
  if (custom) return custom
  return (
    <a
      href={href}
      className="font-medium text-primary underline underline-offset-4"
      onClick={event => {
        event.preventDefault()
        if (href && /^https?:\/\//i.test(href)) {
          if (!isDesktop()) {
            window.open(href, '_blank', 'noopener,noreferrer')
            return
          }
          openUrl(href).catch((error: unknown) => {
            logger.warn('Failed to open link', { href, error })
          })
        }
      }}
    >
      {children}
    </a>
  )
}

/**
 * Agent output is untrusted (it can echo prompt-injected tool output), so:
 * - raw HTML is never rendered (react-markdown's default);
 * - links open in the system browser, never inside the app webview
 *   (in-app `nandi://request/` links are only ever shown as Request cards);
 * - images are not loaded, so a crafted URL can't exfiltrate data.
 */
const COMPONENTS: Components = {
  a: ({ href, children }) => (
    <MarkdownLink href={href}>{children}</MarkdownLink>
  ),
  img: ({ alt, src }) => (
    <span className="text-muted-foreground" title={src}>
      [{alt || 'image'}]
    </span>
  ),
  pre: ({ children }) => (
    <pre
      dir="ltr"
      className="overflow-x-auto rounded-md border bg-muted/60 px-3 py-2 text-start font-mono text-xs"
    >
      {children}
    </pre>
  ),
  code: ({ className, children }) => (
    <code
      className={
        className ?? 'rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]'
      }
    >
      {children}
    </code>
  ),
  ul: ({ children }) => <ul className="list-disc ps-5">{children}</ul>,
  ol: ({ children }) => <ol className="list-decimal ps-5">{children}</ol>,
  h1: ({ children }) => <h3 className="text-base font-semibold">{children}</h3>,
  h2: ({ children }) => <h3 className="text-base font-semibold">{children}</h3>,
  h3: ({ children }) => <h4 className="text-sm font-semibold">{children}</h4>,
  table: ({ children }) => (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-xs">{children}</table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border px-2 py-1 text-start font-medium">{children}</th>
  ),
  td: ({ children }) => <td className="border px-2 py-1">{children}</td>,
}

export function MarkdownContent({ content }: { content: string }) {
  return (
    <div className="space-y-3 text-sm leading-relaxed text-foreground/90 break-words">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={COMPONENTS}
        urlTransform={url =>
          IN_APP_LINK.test(url) ? url : defaultUrlTransform(url)
        }
      >
        {content}
      </ReactMarkdown>
    </div>
  )
}
