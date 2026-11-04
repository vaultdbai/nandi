/**
 * Opens Settings (a full page) on a given section, so every "Add a model"
 * lands on Models and every "View in logs" on Logs.
 */
import {
  useNavigationStore,
  type AuditLogTab,
  type View,
} from '@/store/navigation-store'

/** The sections of Settings, in rail order. `models` is Models & runtimes. */
export const SETTINGS_SECTIONS = [
  'models',
  'account',
  'cloud',
  'internet',
  'backup',
  'sources',
  'logs',
  'appearance',
] as const

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

export const DEFAULT_SETTINGS_SECTION: SettingsSection = 'models'

export function isSettingsSection(value: unknown): value is SettingsSection {
  return (
    typeof value === 'string' &&
    (SETTINGS_SECTIONS as readonly string[]).includes(value)
  )
}

export interface SettingsOptions {
  /** Scroll to and highlight one item: a provider, a runtime id, a URL */
  focus?: string
  logTab?: AuditLogTab
  /** Internet access filtered to one agent */
  agentId?: string
}

/**
 * The Settings view for `section` (default: the last section used this
 * session, else Models)
 */
export function settingsView(
  section?: SettingsSection,
  options: SettingsOptions = {}
): Extract<View, { kind: 'settings' }> {
  const view: Extract<View, { kind: 'settings' }> = { kind: 'settings' }
  if (section) view.section = section
  if (options.focus) view.focus = options.focus
  if (options.logTab) view.logTab = options.logTab
  if (options.agentId) view.agentId = options.agentId
  return view
}

export function openSettings(
  section?: SettingsSection,
  options?: SettingsOptions
) {
  useNavigationStore.getState().navigate(settingsView(section, options))
}
