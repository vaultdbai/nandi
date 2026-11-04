import { createContext, type ReactNode } from 'react'

/**
 * Lets a surface render some links its own way (the companion shows
 * `nandi://request/<id>` as a live Request card). Returns `null` to fall
 * back to a plain link.
 */
export const MarkdownLinkContext = createContext<
  ((href: string, children: ReactNode) => ReactNode | null) | null
>(null)
