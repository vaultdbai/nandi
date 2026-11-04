/**
 * The Bedrock model catalog and the LiteLLM gateway (TanStack Query),
 * refetched on `bedrock://models_refreshed` / `bedrock://gateway_status`.
 * Also the selector's pure helpers.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { BedrockModel, LitellmGatewayStatus } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const bedrockQueryKeys = {
  all: ['bedrock'] as const,
  models: (region: string | null) =>
    ['bedrock', 'models', region ?? '*'] as const,
  gateway: ['bedrock', 'gateway'] as const,
}

function useBedrockEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'bedrock://models_refreshed' ||
        event.event === 'bedrock://gateway_status' ||
        event.event === 'nandi://resync'
      ) {
        void queryClient.invalidateQueries({ queryKey: bedrockQueryKeys.all })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

/** Cached models for `region` (`null`: the AWS default region) */
export function useBedrockModels(region: string | null) {
  useBedrockEvents()
  return useQuery({
    queryKey: bedrockQueryKeys.models(region),
    queryFn: async (): Promise<BedrockModel[]> =>
      unwrapResult(await client.listBedrockModels(region)),
  })
}

/** Asks Bedrock for its current catalog */
export function useRefreshBedrockModels() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (region: string | null): Promise<BedrockModel[]> =>
      unwrapResult(await client.refreshBedrockModels(region)),
    onSuccess: (models, region) =>
      queryClient.setQueryData(bedrockQueryKeys.models(region), models),
  })
}

export function useLitellmGateway() {
  useBedrockEvents()
  return useQuery({
    queryKey: bedrockQueryKeys.gateway,
    queryFn: async (): Promise<LitellmGatewayStatus> =>
      unwrapResult(await client.getLitellmGatewayStatus()),
  })
}

/** `model` starts (or restarts) the gateway for it; `null` stops it */
export function useSetLitellmGateway() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (model: string | null): Promise<LitellmGatewayStatus> =>
      unwrapResult(
        model === null
          ? await client.stopLitellmGateway()
          : await client.startLitellmGateway(model)
      ),
    onSuccess: status =>
      queryClient.setQueryData(bedrockQueryKeys.gateway, status),
  })
}

export interface BedrockModelGroups {
  /** Native in Claude Code */
  anthropic: BedrockModel[]
  /** Through the LiteLLM gateway */
  thirdParty: BedrockModel[]
}

/**
 * Models matching `query` (name, ID, or provider), split into Anthropic
 * and third-party, each sorted by name; unsupported models are left out
 */
export function groupBedrockModels(
  models: BedrockModel[],
  query: string
): BedrockModelGroups {
  const needle = query.trim().toLowerCase()
  const matches = models
    .filter(model => model.tested !== 'unsupported')
    .filter(
      model =>
        !needle ||
        model.name.toLowerCase().includes(needle) ||
        model.id.toLowerCase().includes(needle) ||
        model.provider.toLowerCase().includes(needle)
    )
    .sort((a, b) => a.name.localeCompare(b.name))
  return {
    anthropic: matches.filter(model => model.isAnthropic),
    thirdParty: matches.filter(model => !model.isAnthropic),
  }
}
