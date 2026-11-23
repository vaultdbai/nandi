import { describe, expect, it } from 'vitest'
import type { NandiBrowser } from '@/lib/bindings'
import {
  fromChoice,
  nandiBrowserState,
  profileChoices,
  toChoice,
} from './browser-profile'

const browser = (overrides: Partial<NandiBrowser> = {}): NandiBrowser => ({
  browser: 'chrome',
  hasProfile: false,
  openInWindow: false,
  inUseBy: null,
  agents: [],
  ...overrides,
})

describe('browser profile choices', () => {
  it('offers the agent its own profile first, then each installed browser', () => {
    expect(profileChoices(['chrome', 'edge'], null)).toEqual([
      'own',
      'chrome',
      'edge',
    ])
    expect(profileChoices([], undefined)).toEqual(['own'])
  })

  it('keeps a granted browser listed after it was uninstalled', () => {
    expect(profileChoices(['chrome'], 'edge')).toEqual([
      'own',
      'chrome',
      'edge',
    ])
  })

  it('maps a choice to the grant and back', () => {
    expect(toChoice(null)).toBe('own')
    expect(toChoice('edge')).toBe('edge')
    expect(fromChoice('own')).toBeNull()
    expect(fromChoice('chrome')).toBe('chrome')
    expect(fromChoice('firefox')).toBeNull()
  })
})

describe('nandiBrowserState', () => {
  it('says who has it open before whether it is signed in', () => {
    expect(nandiBrowserState(browser())).toBe('empty')
    expect(nandiBrowserState(browser({ hasProfile: true }))).toBe('signedIn')
    expect(
      nandiBrowserState(browser({ hasProfile: true, openInWindow: true }))
    ).toBe('open')
    expect(
      nandiBrowserState(browser({ hasProfile: true, inUseBy: 'cos' }))
    ).toBe('inUse')
  })
})
