/**
 * The sites an agent may reach, in the terms the Settings tab shows them:
 * no internet, only some sites, or any site (`*`). Pure helpers, so
 * they're testable without React.
 */
import type { EgressLogEntry } from '@/lib/tauri-bindings'

/** The host rule that matches every site */
export const ANY_SITE = '*'

export type AccessMode = 'none' | 'some' | 'any'

export function accessMode(sites: string[]): AccessMode {
  const listed = sites.map(site => site.trim()).filter(Boolean)
  if (listed.includes(ANY_SITE)) return 'any'
  return listed.length > 0 ? 'some' : 'none'
}

/** `API.Example.com.` → `api.example.com`, as the backend stores it */
export function normalizeSite(site: string): string {
  return site.trim().replace(/\.$/, '').toLowerCase()
}

/** `sites` plus `site`, unless it's there already or every site is allowed */
export function withSite(sites: string[], site: string): string[] {
  const next = normalizeSite(site)
  const listed = sites.map(normalizeSite)
  if (!next || listed.includes(next) || listed.includes(ANY_SITE)) return sites
  return [...sites, next]
}

/**
 * The site a blocked request tried to reach, as an allow rule: the host
 * of a fetch (`GET https://host/path`), or of a tunnel (`host:443`), with
 * the port only when it isn't the default. `null` for AWS calls and
 * requests that weren't blocked.
 */
export function blockedSite(entry: EgressLogEntry): string | null {
  if (entry.allowed) return null
  if (entry.kind === 'fetch') {
    const url = entry.target.split(' ').at(-1) ?? ''
    try {
      return normalizeSite(new URL(url).host) || null
    } catch {
      return null
    }
  }
  if (entry.kind === 'tunnel') {
    const target = normalizeSite(entry.target)
    const colon = target.lastIndexOf(':')
    if (colon === -1) return target || null
    const host = target.slice(0, colon)
    return target.slice(colon + 1) === '443' ? host || null : target
  }
  return null
}
