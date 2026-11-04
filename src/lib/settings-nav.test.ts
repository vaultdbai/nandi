import { beforeEach, describe, expect, it } from 'vitest'
import { useNavigationStore } from '@/store/navigation-store'
import { isSettingsSection, openSettings } from './settings-nav'

describe('openSettings', () => {
  beforeEach(() => {
    useNavigationStore.setState({ current: { kind: 'home' }, history: [] })
  })

  it('opens the Settings page on a section, with options', () => {
    openSettings('logs', { logTab: 'egress' })
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'logs',
      logTab: 'egress',
    })
    openSettings('models', { focus: 'anthropic' })
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'models',
      focus: 'anthropic',
    })
  })

  it('leaves the section to the page when none is given', () => {
    openSettings()
    expect(useNavigationStore.getState().current).toEqual({ kind: 'settings' })
  })

  it('knows its sections', () => {
    expect(isSettingsSection('cloud')).toBe(true)
    expect(isSettingsSection('identity')).toBe(false)
    expect(isSettingsSection(3)).toBe(false)
  })
})
