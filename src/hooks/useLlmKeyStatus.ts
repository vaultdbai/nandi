import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { KeyedProvider } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Shared key so saving a key in Preferences refreshes every consumer */
export const llmKeyStatusQueryKey = ['llmKeyStatus'] as const

/** Which LLM providers have an API key configured (never the keys). */
export function useLlmKeyStatus() {
  return useQuery({
    queryKey: llmKeyStatusQueryKey,
    queryFn: async () => unwrapResult(await client.getLlmKeyStatus()),
  })
}

/**
 * The default model and the model calls actually use: the default when
 * its key is set, else one from a provider that has a key. Lives under
 * the key-status key so saving a key refreshes it too.
 */
export function useDefaultModel() {
  return useQuery({
    queryKey: [...llmKeyStatusQueryKey, 'defaultModel'],
    queryFn: async () => unwrapResult(await client.getDefaultModel()),
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
  return useQuery({
    queryKey: ollamaQueryKey,
    queryFn: async () => unwrapResult(await client.detectOllama()),
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
  return useQuery({
    queryKey: [...llmKeyStatusQueryKey, 'trialAccount'],
    queryFn: async () => unwrapResult(await client.getFreeTrialAccount()),
    enabled,
    retry: false,
    staleTime: 60_000,
  })
}
