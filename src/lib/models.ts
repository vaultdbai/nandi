/**
 * The model providers Nandi can call, shared by onboarding and Settings ›
 * Models. Order matches the backend's fallback (`FALLBACK_MODELS` in
 * `services/llm_keys.rs`): Anthropic, then OpenAI, then DeepSeek. Ollama
 * runs locally and needs no key.
 */
import type {
  KeyedProvider,
  KeyTestFailure,
  LlmKeyStatus,
  LlmKeyTest,
} from '@/lib/bindings'

export interface KeyedProviderInfo {
  provider: KeyedProvider
  /** The model the provider's "Use" button sets */
  defaultModel: string
  /** Models offered for this provider, best first */
  models: string[]
  /** Where to get a key */
  keyUrl: string
}

export const KEYED_PROVIDERS: KeyedProviderInfo[] = [
  {
    provider: 'anthropic',
    defaultModel: 'claude:claude-sonnet-5',
    models: ['claude:claude-sonnet-5', 'claude:claude-opus-5'],
    keyUrl: 'https://console.anthropic.com/settings/keys',
  },
  {
    provider: 'openAi',
    defaultModel: 'gpt:gpt-5',
    models: ['gpt:gpt-5'],
    keyUrl: 'https://platform.openai.com/api-keys',
  },
  {
    provider: 'deepSeek',
    defaultModel: 'deepseek:deepseek-flash',
    models: ['deepseek:deepseek-flash', 'deepseek:deepseek-chat'],
    keyUrl: 'https://platform.deepseek.com/api_keys',
  },
]

/** Suggestions for the default model (any `<provider>:<model>` works) */
export const MODEL_SUGGESTIONS = [
  ...KEYED_PROVIDERS.flatMap(info => info.models),
  'ollama:llama3.2',
]

export function providerInfo(provider: KeyedProvider): KeyedProviderInfo {
  const info = KEYED_PROVIDERS.find(p => p.provider === provider)
  if (!info) throw new Error(`Unknown provider ${provider}`)
  return info
}

/** Providers with a key in the keychain, in fallback order */
export function providersWithKeys(
  statuses: LlmKeyStatus[] | undefined
): KeyedProvider[] {
  return KEYED_PROVIDERS.map(info => info.provider).filter(
    provider =>
      statuses?.some(s => s.provider === provider && s.configured) ?? false
  )
}

const MODEL_LABELS: Record<string, string> = {
  'claude:claude-sonnet-5': 'Claude Sonnet 5',
  'claude:claude-opus-5': 'Claude Opus 5',
  'gpt:gpt-5': 'GPT-5',
  'deepseek:deepseek-flash': 'DeepSeek Flash',
  'deepseek:deepseek-chat': 'DeepSeek Chat',
}

/** A model's name in plain words: `claude:claude-sonnet-5` → `Claude Sonnet 5` */
export function modelLabel(model: string): string {
  const known = MODEL_LABELS[model]
  if (known) return known
  const colon = model.indexOf(':')
  return colon === -1 ? model : model.slice(colon + 1)
}

/** `ollama:<name>` for a model Ollama lists (`llama3.2:latest`) */
export function ollamaModelRef(name: string): string {
  return `ollama:${name.replace(/:latest$/, '')}`
}

/**
 * The provider calls would switch to if `removed`'s key went away: the
 * next one with a key (in fallback order), else Ollama when it runs, else
 * `null` (nothing would work).
 */
export function fallbackAfterRemoving(
  removed: KeyedProvider,
  statuses: LlmKeyStatus[] | undefined,
  ollamaRunning: boolean
): KeyedProvider | 'ollama' | null {
  const next = providersWithKeys(statuses).find(p => p !== removed)
  if (next) return next
  return ollamaRunning ? 'ollama' : null
}

/** What a key check means, as one plain-words outcome */
export type KeyTestOutcome = 'works' | KeyTestFailure

export function keyTestOutcome(test: LlmKeyTest): KeyTestOutcome {
  return test.ok ? 'works' : (test.failure ?? 'other')
}

/** Who a model is called through: a keyed provider, or Ollama here */
export type ModelProvider = KeyedProvider | 'ollama'

/** In the order the picker offers them (the backend's fallback order) */
export const MODEL_PROVIDERS: ModelProvider[] = [
  ...KEYED_PROVIDERS.map(info => info.provider),
  'ollama',
]

const PREFIXES: Record<ModelProvider, string> = {
  anthropic: 'claude',
  openAi: 'gpt',
  deepSeek: 'deepseek',
  ollama: 'ollama',
}

/** `claude:claude-sonnet-5` → Anthropic and `claude-sonnet-5` */
export function splitModel(model: string): {
  provider: ModelProvider | null
  name: string
} {
  const colon = model.indexOf(':')
  return colon === -1
    ? { provider: null, name: model }
    : { provider: providerOf(model), name: model.slice(colon + 1) }
}

/** The ref the backend takes: `<prefix>:<name>` */
export function joinModel(provider: ModelProvider, name: string): string {
  return `${PREFIXES[provider]}:${name.trim()}`
}

/**
 * The model a provider starts with when picked: its usual one, or for
 * Ollama the first model installed here
 */
export function defaultModelName(
  provider: ModelProvider,
  ollamaModels: readonly string[]
): string {
  if (provider === 'ollama') {
    const first = ollamaModels[0]
    return first ? splitModel(ollamaModelRef(first)).name : 'llama3.2'
  }
  return splitModel(providerInfo(provider).defaultModel).name
}

/**
 * Names offered while typing: hints only, since any model the provider
 * takes works (models change faster than Nandi is released)
 */
export function modelSuggestions(
  provider: ModelProvider,
  ollamaModels: readonly string[]
): string[] {
  const refs =
    provider === 'ollama'
      ? ollamaModels.map(ollamaModelRef)
      : providerInfo(provider).models
  return refs.map(ref => splitModel(ref).name)
}

/** Its key isn't set (a signed-in Claude Code stands in for Anthropic's) */
export function providerNeedsKey(
  provider: ModelProvider,
  statuses: LlmKeyStatus[] | undefined,
  claudeSignedIn: boolean
): boolean {
  if (provider === 'ollama') return false
  if (provider === 'anthropic' && claudeSignedIn) return false
  return !providersWithKeys(statuses).includes(provider)
}

/** The provider a `<provider>:<model>` ref calls */
export function providerOf(model: string): ModelProvider | null {
  const prefix = model.split(':', 1)[0]
  switch (prefix) {
    case 'claude':
      return 'anthropic'
    case 'gpt':
      return 'openAi'
    case 'deepseek':
      return 'deepSeek'
    case 'ollama':
      return 'ollama'
    default:
      return null
  }
}

/** Why calls don't use the chosen model, as an i18n key under `models.using.` */
export type ModelReason =
  | { key: 'noModel' }
  | { key: 'freeTokens'; model: string }
  | { key: 'noKeyFor'; provider: KeyedProvider; model: string }
  | { key: 'instead'; model: string }

/**
 * The line under the picker: nothing when calls use the chosen model;
 * else which model they use and why (free tokens, no key for the chosen
 * provider, or no model at all)
 */
export function modelReason({
  defaultModel,
  effectiveModel,
  onFreeTokens,
}: {
  defaultModel: string
  effectiveModel: string | null
  onFreeTokens: boolean
}): ModelReason | null {
  if (effectiveModel === null) return { key: 'noModel' }
  if (onFreeTokens) return { key: 'freeTokens', model: effectiveModel }
  if (effectiveModel === defaultModel) return null
  const provider = providerOf(defaultModel)
  return provider && provider !== 'ollama'
    ? { key: 'noKeyFor', provider, model: effectiveModel }
    : { key: 'instead', model: effectiveModel }
}
