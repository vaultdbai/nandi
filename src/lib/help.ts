/**
 * The built-in help guide's links (`services::help`, `guide.json`): a
 * target is a place in the app (`settings:computer`, `network:groups`,
 * `agents`) or a web page (`url:https://...`). Topic words live in the
 * locales under `help.topics.<id>`.
 */
import type { View } from '@/store/navigation-store'
import { isSettingsSection, settingsView } from '@/lib/settings-nav'

const NETWORK_TABS = ['people', 'groups', 'sharing', 'connection'] as const
type NetworkTab = (typeof NETWORK_TABS)[number]

function isNetworkTab(value: string | undefined): value is NetworkTab {
  return (NETWORK_TABS as readonly (string | undefined)[]).includes(value)
}

export type HelpDestination =
  { kind: 'view'; view: View } | { kind: 'url'; url: string }

/** Where a link goes, or `null` for a target this build doesn't know */
export function helpDestination(target: string): HelpDestination | null {
  if (target.startsWith('url:')) {
    const url = target.slice('url:'.length)
    return url.startsWith('https://') ? { kind: 'url', url } : null
  }
  const [place, sub, focus] = target.split(':')
  switch (place) {
    case 'settings':
      if (sub === undefined) return { kind: 'view', view: settingsView() }
      return isSettingsSection(sub)
        ? {
            kind: 'view',
            view: settingsView(sub, focus ? { focus } : {}),
          }
        : null
    case 'network':
      return isNetworkTab(sub)
        ? { kind: 'view', view: { kind: 'network', tab: sub } }
        : null
    case 'agents':
    case 'library':
    case 'automations':
    case 'inbox':
      return { kind: 'view', view: { kind: place } }
    case 'home':
      return { kind: 'view', view: { kind: 'home' } }
    default:
      return null
  }
}

/** The locale keys of a topic's words */
export function helpTopicKeys(id: string) {
  return {
    title: `help.topics.${id}.title`,
    answer: `help.topics.${id}.answer`,
  }
}
