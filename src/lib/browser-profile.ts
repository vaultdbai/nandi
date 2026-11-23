/**
 * My Nandi browser, as the views show it: the shared profile per browser
 * type (Settings › Browser) and an agent's choice between its own profile
 * and one of them (the agent's page). Pure: no calls, no state.
 */
import type { BrowserKind, NandiBrowser } from '@/lib/bindings'

/** Product names, not translated */
export const BROWSER_LABELS: Record<BrowserKind, string> = {
  chrome: 'Chrome',
  edge: 'Edge',
}

/** `own`: the agent's own profile; else My Nandi browser of that type */
export type ProfileChoice = 'own' | BrowserKind

function isBrowserKind(value: string): value is BrowserKind {
  return value in BROWSER_LABELS
}

/**
 * What the agent's browser may use: its own profile, then each installed
 * browser's shared one. The current grant stays listed even if that
 * browser is gone, so the view shows the truth.
 */
export function profileChoices(
  installed: readonly BrowserKind[],
  current: BrowserKind | null | undefined
): ProfileChoice[] {
  const kinds: BrowserKind[] = [...installed]
  if (current && !kinds.includes(current)) kinds.push(current)
  return ['own', ...kinds]
}

export function toChoice(kind: BrowserKind | null | undefined): ProfileChoice {
  return kind ?? 'own'
}

/** The grant a choice stands for: `null` is the agent's own profile */
export function fromChoice(choice: string): BrowserKind | null {
  return isBrowserKind(choice) ? choice : null
}

/** One line of Settings › Browser: who has it, else whether it's signed in */
export type NandiBrowserState = 'inUse' | 'open' | 'signedIn' | 'empty'

export function nandiBrowserState(browser: NandiBrowser): NandiBrowserState {
  if (browser.inUseBy) return 'inUse'
  if (browser.openInWindow) return 'open'
  return browser.hasProfile ? 'signedIn' : 'empty'
}
