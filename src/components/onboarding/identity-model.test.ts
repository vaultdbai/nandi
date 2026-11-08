import { describe, expect, it } from 'vitest'
import type { EmailProviderStatus, IdentityStatus } from '@/lib/bindings'
import {
  branchFor,
  DEFAULT_RELAY_DRAFT,
  draftFromMailbox,
  mailboxIdentity,
  isRelayUrl,
  isUnencryptedRemoteRelay,
  needsOnboarding,
  publishesGateway,
  relayDraftFrom,
  relayHost,
  relayNeedsSave,
  skipTurnsRelayOff,
  resumeStep,
  slugify,
  validateName,
  validateSlug,
} from './identity-model'

describe('slugify', () => {
  it('lowercases and hyphenates', () => {
    expect(slugify('Ada Lovelace')).toBe('ada-lovelace')
  })

  it('drops accents and collapses runs of other characters', () => {
    expect(slugify('  Zoë   Ündërwood!!  ')).toBe('zoe-underwood')
    expect(slugify('a__b--c')).toBe('a-b-c')
  })

  it('trims hyphens and caps at 32 characters', () => {
    const slug = slugify(`${'x'.repeat(31)} yz`)
    expect(slug.length).toBeLessThanOrEqual(32)
    expect(slug.endsWith('-')).toBe(false)
    expect(slugify('---')).toBe('')
  })

  it('leaves non-Latin names empty', () => {
    expect(slugify('محمد')).toBe('')
  })
})

describe('validation', () => {
  it('names are 1–64 characters', () => {
    expect(validateName('')).toBe('required')
    expect(validateName('   ')).toBe('required')
    expect(validateName('A')).toBeNull()
    expect(validateName('a'.repeat(65))).toBe('tooLong')
  })

  it('slugs follow the core pattern, one character allowed', () => {
    expect(validateSlug('')).toBe('required')
    expect(validateSlug('a')).toBeNull()
    expect(validateSlug('ada-lovelace')).toBeNull()
    expect(validateSlug('-ada')).toBe('invalid')
    expect(validateSlug('ada-')).toBe('invalid')
    expect(validateSlug('Ada')).toBe('invalid')
    expect(validateSlug('a'.repeat(32))).toBeNull()
    expect(validateSlug('a'.repeat(33))).toBe('invalid')
  })

  it('branch and relay helpers', () => {
    expect(branchFor('ada')).toBe('nandi/ada')
    expect(branchFor('ada', 'team')).toBe('team/ada')
    expect(isRelayUrl('wss://relay.nandi.dev/relay')).toBe(true)
    expect(isRelayUrl('https://relay.nandi.dev')).toBe(false)
  })

  it('flags plain ws:// relays on other machines only', () => {
    expect(isUnencryptedRemoteRelay('ws://relay.example:8787/relay')).toBe(true)
    expect(
      isUnencryptedRemoteRelay('ws://192.168.1.10:8787/relay?token=x')
    ).toBe(true)
    expect(isUnencryptedRemoteRelay('ws://localhost:8787/relay')).toBe(false)
    expect(isUnencryptedRemoteRelay('ws://127.0.0.1:8787/relay')).toBe(false)
    expect(isUnencryptedRemoteRelay('ws://[::1]:8787/relay')).toBe(false)
    expect(isUnencryptedRemoteRelay('wss://relay.example/relay')).toBe(false)
    expect(isUnencryptedRemoteRelay('')).toBe(false)
  })
})

/** The relay this (test) build comes with */
const BUILT_IN = 'wss://relay.example.org/relay'

describe('configured relay', () => {
  it('preselects the relay already configured', () => {
    expect(relayDraftFrom(null)).toBeNull()
    expect(relayDraftFrom(undefined)).toBeNull()
    expect(relayDraftFrom('  ')).toBeNull()
    expect(relayDraftFrom(BUILT_IN, BUILT_IN)).toEqual({
      choice: 'public',
      customUrl: '',
    })
    // The build's relay with this instance's token is still the build's
    expect(relayDraftFrom(`${BUILT_IN}?token=t`, BUILT_IN)?.choice).toBe(
      'public'
    )
    // Without a build relay, any URL is the user's own
    expect(relayDraftFrom(BUILT_IN)?.choice).toBe('custom')
    expect(relayDraftFrom(' ws://127.0.0.1:8787/relay ')).toEqual({
      choice: 'custom',
      customUrl: 'ws://127.0.0.1:8787/relay',
    })
  })

  it('saves (and so reconnects) only when the URL changes', () => {
    const custom = { choice: 'custom' as const, customUrl: 'ws://a.test/r' }
    const pub = { choice: 'public' as const, customUrl: '' }
    const skip = { choice: 'skip' as const, customUrl: '' }

    expect(relayNeedsSave(pub, null, BUILT_IN)).toBe(true)
    expect(relayNeedsSave(pub, BUILT_IN, BUILT_IN)).toBe(false)
    expect(relayNeedsSave(pub, `${BUILT_IN}?token=t`, BUILT_IN)).toBe(false)
    expect(relayNeedsSave(pub, 'ws://a.test/r', BUILT_IN)).toBe(true)
    // No build relay: nothing to join, nothing to save
    expect(relayNeedsSave(pub, null)).toBe(false)
    expect(relayNeedsSave(custom, 'ws://a.test/r')).toBe(false)
    expect(
      relayNeedsSave(
        { ...custom, customUrl: ' ws://a.test/r ' },
        'ws://a.test/r'
      )
    ).toBe(false)
    expect(relayNeedsSave(custom, BUILT_IN)).toBe(true)
    expect(relayNeedsSave(skip, null)).toBe(false)
    expect(relayNeedsSave(skip, 'ws://a.test/r')).toBe(false)
  })

  it('"Not now" leaves only a relay joined on this step', () => {
    const skip = { choice: 'skip' as const, customUrl: '' }
    const pub = { choice: 'public' as const, customUrl: '' }
    // The build's relay, joined before: left alone
    expect(skipTurnsRelayOff(skip, 'builtIn', false)).toBe(false)
    // Joined on this step, then the user chose Not now
    expect(skipTurnsRelayOff(skip, 'config', true)).toBe(true)
    // A relay set up before, or by NANDI_RELAY_URL, stays
    expect(skipTurnsRelayOff(skip, 'config', false)).toBe(false)
    expect(skipTurnsRelayOff(skip, 'env', true)).toBe(false)
    expect(skipTurnsRelayOff(skip, null, false)).toBe(false)
    // Only skipping turns it off
    expect(skipTurnsRelayOff(pub, 'builtIn', true)).toBe(false)
  })
})

describe('gateway publishing', () => {
  it('publishes only when a relay is chosen and reachability is ticked', () => {
    // Off by default: being reachable is always a choice
    expect(publishesGateway({ choice: 'public', customUrl: '' })).toBe(false)
    expect(
      publishesGateway({ choice: 'public', customUrl: '', reachable: true })
    ).toBe(true)
    expect(
      publishesGateway({
        choice: 'custom',
        customUrl: 'ws://a',
        reachable: true,
      })
    ).toBe(true)
    expect(
      publishesGateway({ choice: 'skip', customUrl: '', reachable: true })
    ).toBe(false)
  })

  it('starts the network step on Not now', () => {
    expect(DEFAULT_RELAY_DRAFT.choice).toBe('skip')
  })
})

describe('relayHost', () => {
  it('names the host only', () => {
    expect(relayHost(BUILT_IN)).toBe('relay.example.org')
    expect(relayHost(' ws://127.0.0.1:8787/relay ')).toBe('127.0.0.1:8787')
    expect(relayHost('not a url')).toBe('not a url')
  })
})

describe('onboarding resume', () => {
  const status = (
    onboarded: boolean,
    onboardingStep: IdentityStatus['onboardingStep']
  ): IdentityStatus => ({
    onboarded,
    displayName: onboarded ? 'Ada' : null,
    userSlug: onboarded ? 'ada' : null,
    branch: null,
    onboardingStep,
  })

  it('runs until a name is saved and setup reaches the end', () => {
    expect(needsOnboarding(status(false, null))).toBe(true)
    expect(needsOnboarding(status(false, 'done'))).toBe(true)
    expect(needsOnboarding(status(true, 'network'))).toBe(true)
    expect(needsOnboarding(status(true, 'done'))).toBe(false)
    // A name with no step recorded yet: setup is still open
    expect(needsOnboarding(status(true, null))).toBe(true)
    expect(needsOnboarding(status(true, undefined))).toBe(true)
  })

  it('resumes at the step reached, but never past an unsaved name', () => {
    // Email is first; Name once the email step was left
    expect(resumeStep(status(false, null))).toBe('email')
    expect(resumeStep(status(false, 'packs'))).toBe('email')
    expect(resumeStep(status(false, 'name'))).toBe('name')
    expect(resumeStep(status(true, null))).toBe('model')
    expect(resumeStep(status(true, 'email'))).toBe('model')
    expect(resumeStep(status(true, 'name'))).toBe('model')
    expect(resumeStep(status(true, 'model'))).toBe('model')
    expect(resumeStep(status(true, 'network'))).toBe('network')
    expect(resumeStep(status(true, 'packs'))).toBe('packs')
  })
})

describe('mailbox identity', () => {
  const provider = (
    overrides: Partial<EmailProviderStatus> = {}
  ): EmailProviderStatus => ({
    provider: 'outlook',
    available: true,
    connected: true,
    accountName: 'Dev Chechi',
    accountEmail: 'dev.chechi@company.com',
    accountProfile: null,
    state: 'ready',
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    backfill: { phase: 'off', backTo: null, read: 0 },
    ...overrides,
  })

  it('is the first connected mailbox that names its owner', () => {
    expect(mailboxIdentity([])).toBeNull()
    expect(mailboxIdentity([provider({ connected: false })])).toBeNull()
    expect(mailboxIdentity([provider({ accountName: null })])).toBeNull()
    expect(
      mailboxIdentity([
        provider({ accountName: '  ' }),
        provider({ provider: 'gmail', accountName: 'Dev G' }),
      ])
    ).toMatchObject({ provider: 'gmail', name: 'Dev G' })
  })

  it('carries title, department and manager when the directory gave them', () => {
    expect(
      mailboxIdentity([
        provider({
          accountProfile: {
            title: 'Platform lead',
            department: 'Engineering',
            manager: { name: 'Priya Rao', email: null, title: 'VP' },
          },
        }),
      ])
    ).toEqual({
      provider: 'outlook',
      name: 'Dev Chechi',
      email: 'dev.chechi@company.com',
      title: 'Platform lead',
      department: 'Engineering',
      manager: 'Priya Rao (VP)',
    })
    expect(mailboxIdentity([provider()])).toMatchObject({
      title: null,
      department: null,
      manager: null,
    })
    expect(
      mailboxIdentity([
        provider({
          accountProfile: {
            title: null,
            department: null,
            manager: { name: 'Sam', email: null, title: null },
          },
        }),
      ])?.manager
    ).toBe('Sam')
  })

  it('makes the handle from the address, and falls back to the name', () => {
    const identity = mailboxIdentity([provider()])
    if (!identity) throw new Error('expected a mailbox identity')
    expect(draftFromMailbox(identity)).toEqual({
      name: 'Dev Chechi',
      slug: 'dev-chechi',
      slugEdited: true,
    })
    expect(draftFromMailbox({ ...identity, email: '+++@company.com' })).toEqual(
      { name: 'Dev Chechi', slug: 'dev-chechi', slugEdited: false }
    )
  })
})
