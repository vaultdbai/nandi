import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { KeyedProvider } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useClientQuery } from '@/hooks/useClientQuery'
import { modelReason } from '@/lib/models'

/** Shared key so saving a key in Preferences refreshes every consumer */
export const llmKeyStatusQueryKey = ['llmKeyStatus'] as const

/** Which LLM providers have an API key configured (never the keys). */
export function useLlmKeyStatus() {
  return useClientQuery(llmKeyStatusQueryKey, () => client.getLlmKeyStatus())
}

/**
 * The default model and the model calls actually use: the default when
 * its key is set, else one from a provider that has a key. Lives under
 * the key-status key so saving a key refreshes it too.
 */
export function useDefaultModel() {
  return useClientQuery([...llmKeyStatusQueryKey, 'defaultModel'], () =>
    client.getDefaultModel()
  )
}

/**
 * Which model answers a call set to `model` (an agent's own) with the keys
 * set now, and why when it's another one (`modelReason`). Nothing for no
 * model (the default applies: Settings › Models says it). Under the
 * key-status key, so saving a key refreshes it.
 */
export function useModelReason(model: string | null) {
  const effective = useClientQuery(
    [...llmKeyStatusQueryKey, 'effective', model],
    () => client.effectiveModel(model ?? ''),
    { enabled: Boolean(model), retry: false }
  )
  if (!model || !effective.data) return null
  return modelReason({
    chosenModel: model,
    effectiveModel: effective.data.model,
    onFreeTokens: effective.data.onTrial,
  })
}

/**
 * The provider's own current model list (fetched with its stored key),
 * for the default-model suggestions. Only asked when the provider has a
 * key; lives under the key-status key so saving a key refetches it. A
 * failure leaves it empty, and the field keeps its built-in suggestions.
 */
export function useProviderModels(
  provider: KeyedProvider | null,
  hasKey: boolean
) {
  return useQuery({
    queryKey: [...llmKeyStatusQueryKey, 'models', provider],
    queryFn: async () =>
      provider ? unwrapResult(await client.listProviderModels(provider)) : [],
    enabled: provider !== null && hasKey,
    staleTime: 10 * 60_000,
    retry: false,
  })
}

export const ollamaQueryKey = ['ollama'] as const

/** Whether Ollama runs on this computer (`refetch` is "Check again") */
export function useOllama() {
  return useClientQuery(ollamaQueryKey, () => client.detectOllama(), {
    retry: false,
    staleTime: 30_000,
  })
}

/**
 * The free trial's allowance as Nandi's trial gateway counts it (used,
 * limit, paused, the account this computer is linked to). Asked only
 * while the trial is shown; it needs the internet, so a failure is shown
 * as "can't reach", not retried.
 */
export function useFreeTrialAccount(enabled: boolean) {
  return useClientQuery(
    [...llmKeyStatusQueryKey, 'trialAccount'],
    () => client.getFreeTrialAccount(),
    { enabled, retry: false, staleTime: 60_000 }
  )
}
