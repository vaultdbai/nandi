/**
 * Pure rules for the local identity (display name + handle, the core's
 * "user slug") and the relay choices onboarding offers. Mirrors the core's
 * validation (`services/auth/mod.rs`), so the forms can say what's wrong
 * before saving.
 */
import type {
  EmailProviderStatus,
  IdentityStatus,
  MailProvider,
  RelayUrlSource,
} from '@/lib/bindings'

export const NAME_MAX_LENGTH = 64
export const SLUG_MAX_LENGTH = 32

/** `services::remote::DEFAULT_BRANCH_PREFIX` */
export const DEFAULT_BRANCH_PREFIX = 'nandi'

const SLUG_PATTERN = /^(?:[a-z0-9]|[a-z0-9][a-z0-9-]{0,30}[a-z0-9])$/

/**
 * A slug from a display name: lowercase ASCII (accents dropped), runs of
 * anything else become one hyphen, no leading/trailing hyphen, at most 32
 * characters.
 */
export function slugify(name: string): string {
  return name
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, SLUG_MAX_LENGTH)
    .replace(/-+$/, '')
}

export type NameError = 'required' | 'tooLong'
export type SlugError = 'required' | 'invalid'

export function validateName(name: string): NameError | null {
  const trimmed = name.trim()
  if (trimmed.length === 0) return 'required'
  if (trimmed.length > NAME_MAX_LENGTH) return 'tooLong'
  return null
}

export function validateSlug(slug: string): SlugError | null {
  if (slug.length === 0) return 'required'
  if (!SLUG_PATTERN.test(slug)) return 'invalid'
  return null
}

/** `<prefix>/<slug>`, the only branch Nandi pushes to */
export function branchFor(slug: string, prefix = DEFAULT_BRANCH_PREFIX) {
  return `${prefix.trim() || DEFAULT_BRANCH_PREFIX}/${slug || '…'}`
}

/** A relay URL the core can dial */
export function isRelayUrl(url: string): boolean {
  return /^wss?:\/\/[^\s/]+/i.test(url.trim())
}

/**
 * A plain `ws://` relay on another machine: cards, tasks and results cross
 * the network unencrypted. Same rule as the Rust side
 * (`relay::tls::is_unencrypted_remote`).
 */
export function isUnencryptedRemoteRelay(url: string): boolean {
  const match = /^ws:\/\/([^/?]+)/i.exec(url.trim())
  if (!match?.[1]) return false
  const authority = match[1]
  const host = authority.startsWith('[')
    ? (authority.slice(1).split(']')[0] ?? '')
    : (authority.split(':')[0] ?? '')
  const lower = host.toLowerCase()
  const loopback =
    lower === 'localhost' ||
    lower === '::1' ||
    /^127\.\d+\.\d+\.\d+$/.test(lower)
  return !loopback
}

export type RelayChoice = 'public' | 'custom' | 'skip'

export interface RelayDraft {
  choice: RelayChoice
  customUrl: string
  /**
   * The join code for a team network (blank: none, or the relay needs
   * none). Kept in memory only, and never shown once typed
   */
  joinCode?: string
  /** Turn on and list the gateway at the relay (unset: no) */
  reachable?: boolean
}

/**
 * The URL the choice connects to (`null`: no relay). `public` is the relay
 * this build comes with (`RelayStatus.builtInUrl`); without one there's
 * nothing to join.
 */
export function relayUrlFor(
  draft: RelayDraft,
  builtInUrl: string | null = null
): string | null {
  switch (draft.choice) {
    case 'public':
      return builtInUrl
    case 'custom':
      return draft.customUrl.trim()
    case 'skip':
      return null
  }
}

/**
 * The draft the network step starts from while nothing (or nothing yet) is
 * configured: "Not now", so joining a network is always a choice
 */
export const DEFAULT_RELAY_DRAFT: RelayDraft = {
  choice: 'skip',
  customUrl: '',
}

/** `relay.example.com` from `wss://relay.example.com/relay`, for plain copy */
export function relayHost(url: string): string {
  const match = /^wss?:\/\/([^/\s]+)/i.exec(url.trim())
  return match?.[1] ?? url
}

/** A relay URL without its `?token=` (the configured URL may carry one) */
export function withoutToken(url: string): string {
  return url.trim().split('?')[0] ?? ''
}

/**
 * The draft for a relay that's already configured (`NANDI_RELAY_URL`,
 * `config.json`, or the build's own), so onboarding keeps it: `public` for
 * the build's relay, `custom` with the URL otherwise. `null` while none is
 * configured.
 */
export function relayDraftFrom(
  configuredUrl: string | null | undefined,
  builtInUrl: string | null = null
): RelayDraft | null {
  const url = configuredUrl?.trim()
  if (!url) return null
  return builtInUrl && withoutToken(url) === withoutToken(builtInUrl)
    ? { choice: 'public', customUrl: '' }
    : { choice: 'custom', customUrl: url }
}

/**
 * Whether "Not now" has to leave the relay again: this step joined one
 * before the user changed their mind. A relay the user joined before (or
 * `NANDI_RELAY_URL`) is left alone, and the build's own relay is never in
 * use until joined, so there is nothing to turn off.
 */
export function skipTurnsRelayOff(
  draft: RelayDraft,
  configuredSource: RelayUrlSource | null | undefined,
  joinedHere: boolean
): boolean {
  if (draft.choice !== 'skip' || configuredSource === 'env') return false
  return joinedHere
}

/**
 * Whether continuing with `draft` has to join the relay. Joining makes
 * the core reconnect, so the URL already configured is left alone. Skipping
 * saves nothing ([`skipTurnsRelayOff`] says when it turns the relay off).
 */
export function relayNeedsSave(
  draft: RelayDraft,
  configuredUrl: string | null | undefined,
  builtInUrl: string | null = null
): boolean {
  const url = relayUrlFor(draft, builtInUrl)
  if (url === null) return false
  return withoutToken(url) !== withoutToken(configuredUrl ?? '')
}

/**
 * Choosing a relay publishes the gateway there only when the user ticks
 * "Let others send requests to my Nandi"; skipping never does
 */
export function publishesGateway(draft: RelayDraft): boolean {
  return draft.choice !== 'skip' && draft.reachable === true
}

/**
 * Onboarding's steps, in order (the step indicator and resume use it).
 * Email comes first: a connected mailbox fills in the name step.
 */
export const ONBOARDING_STEPS = [
  'email',
  'name',
  'model',
  'computer',
  'network',
  'packs',
] as const
export type OnboardingStepKey = (typeof ONBOARDING_STEPS)[number]

/**
 * Whether first-run setup still has to run. Setup is done only when a name
 * is saved and its last step is recorded as `done`; anything else (no
 * step recorded yet included) keeps it open.
 */
export function needsOnboarding(status: IdentityStatus): boolean {
  return !status.onboarded || status.onboardingStep !== 'done'
}

/**
 * The step to open. Before a name is saved: Name once Email was left
 * (the step recorded is `name`), else Email, which a connected mailbox
 * and a skip both pass. After: the step recorded, Model when none past
 * Name is.
 */
export function resumeStep(status: IdentityStatus): OnboardingStepKey {
  const step = status.onboardingStep
  if (!status.onboarded) return step === 'name' ? 'name' : 'email'
  return !step || step === 'email' || step === 'name' || step === 'done'
    ? 'model'
    : step
}

/** Who the connected mailbox says the user is */
export interface MailboxIdentity {
  provider: MailProvider
  name: string
  email: string
  /** Outlook only: from the directory, each when it has one */
  title: string | null
  department: string | null
  /** "Priya Rao" or "Priya Rao (VP Engineering)" */
  manager: string | null
}

/**
 * The first connected mailbox that names its owner (Outlook is listed
 * first), or `null`: setup then asks, as it does without a mailbox.
 */
export function mailboxIdentity(
  providers: readonly EmailProviderStatus[]
): MailboxIdentity | null {
  for (const status of providers) {
    const name = status.accountName?.trim() ?? ''
    const email = status.accountEmail?.trim() ?? ''
    if (!status.connected || !name || !email) continue
    const profile = status.accountProfile
    const manager = profile?.manager
    return {
      provider: status.provider,
      name,
      email,
      title: profile?.title ?? null,
      department: profile?.department ?? null,
      manager: manager
        ? manager.title
          ? `${manager.name} (${manager.title})`
          : manager.name
        : null,
    }
  }
  return null
}

/**
 * The name step's draft from a mailbox: its name, and the handle from the
 * address's local part (`dev.chechi@…` is `dev-chechi`). A local part that
 * makes no valid handle falls back to one from the name.
 */
export function draftFromMailbox(identity: MailboxIdentity): {
  name: string
  slug: string
  slugEdited: boolean
} {
  const local = slugify(identity.email.split('@')[0] ?? '')
  if (validateSlug(local) === null) {
    return { name: identity.name, slug: local, slugEdited: true }
  }
  return {
    name: identity.name,
    slug: slugify(identity.name),
    slugEdited: false,
  }
}
