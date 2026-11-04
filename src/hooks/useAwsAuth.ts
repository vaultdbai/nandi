/**
 * AWS for Bedrock, from the user's own AWS config (TanStack Query): the
 * AWS CLI, profiles, credential status, the Bedrock API key's presence,
 * and SSO login. No hook ever sees key material.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  AwsCliInfo,
  AwsProfile,
  BedrockApiKeyStatus,
  CredentialStatus,
  KeyProfileInput,
  SsoLoginResult,
  SsoSetup,
  SsoSetupFinish,
  SsoSetupInput,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const awsQueryKeys = {
  all: ['aws'] as const,
  cli: ['aws', 'cli'] as const,
  profiles: ['aws', 'profiles'] as const,
  credentials: (profile: string | null) =>
    ['aws', 'credentials', profile ?? '*'] as const,
  apiKey: ['aws', 'api-key'] as const,
  login: (loginId: string) => ['aws', 'login', loginId] as const,
  setup: (setupId: string) => ['aws', 'setup', setupId] as const,
}

/** How often a pending SSO login is checked */
export const SSO_POLL_MS = 2_000

function useAwsEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'aws://profile_changed' ||
        event.event === 'aws://credentials_expired' ||
        event.event === 'nandi://resync'
      ) {
        void queryClient.invalidateQueries({ queryKey: awsQueryKeys.all })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

export function useAwsCli() {
  return useQuery({
    queryKey: awsQueryKeys.cli,
    queryFn: async (): Promise<AwsCliInfo> =>
      unwrapResult(await client.detectAwsCli()),
    staleTime: 5 * 60_000,
  })
}

export function useAwsProfiles() {
  useAwsEvents()
  return useQuery({
    queryKey: awsQueryKeys.profiles,
    queryFn: async (): Promise<AwsProfile[]> =>
      unwrapResult(await client.listAwsProfiles()),
  })
}

/** Resolves the profile's credentials and asks STS whose they are */
export function useAwsCredentials(profile: string | null, enabled = true) {
  return useQuery({
    queryKey: awsQueryKeys.credentials(profile),
    queryFn: async (): Promise<CredentialStatus> =>
      unwrapResult(await client.checkAwsCredentials(profile, null)),
    enabled,
    retry: false,
  })
}

export function useBedrockApiKeyStatus() {
  return useQuery({
    queryKey: awsQueryKeys.apiKey,
    queryFn: async (): Promise<BedrockApiKeyStatus> =>
      unwrapResult(await client.getAwsBedrockApiKeyStatus()),
  })
}

/** Starts `aws sso login --profile <profile>` (the browser opens) */
export function useStartSsoLogin() {
  return useMutation({
    mutationFn: async (profile: string): Promise<SsoLoginResult> =>
      unwrapResult(await client.startAwsSsoLogin(profile)),
  })
}

/** After a profile is written: the list and every credential check */
function useRefreshProfiles() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: awsQueryKeys.profiles })
    void queryClient.invalidateQueries({ queryKey: ['aws', 'credentials'] })
  }
}

/** Writes an access-key profile through `aws configure` */
export function useSaveKeyProfile() {
  const refresh = useRefreshProfiles()
  return useMutation({
    mutationFn: async (input: KeyProfileInput): Promise<AwsProfile> =>
      unwrapResult(await client.saveAwsKeyProfile(input)),
    onSuccess: refresh,
  })
}

/** Starts the IAM Identity Center sign-in for a new SSO profile */
export function useStartSsoSetup() {
  return useMutation({
    mutationFn: async (input: SsoSetupInput): Promise<SsoSetup> =>
      unwrapResult(await client.startAwsSsoSetup(input)),
  })
}

/** Polls a setup until it's no longer pending (then it lists accounts) */
export function useSsoSetup(setupId: string | null) {
  return useQuery({
    queryKey: awsQueryKeys.setup(setupId ?? ''),
    queryFn: async (): Promise<SsoSetup> =>
      unwrapResult(await client.pollAwsSsoSetup(setupId ?? '')),
    enabled: !!setupId,
    retry: false,
    refetchInterval: q =>
      q.state.data && q.state.data.status !== 'pending' ? false : SSO_POLL_MS,
  })
}

/** Writes the SSO profile for the picked account and role */
export function useFinishSsoSetup() {
  const refresh = useRefreshProfiles()
  return useMutation({
    mutationFn: async (finish: SsoSetupFinish): Promise<AwsProfile> =>
      unwrapResult(await client.finishAwsSsoSetup(finish)),
    onSuccess: refresh,
  })
}

/** Polls a login until it's no longer pending; then refreshes credentials */
export function useSsoLogin(loginId: string | null) {
  const queryClient = useQueryClient()
  const query = useQuery({
    queryKey: awsQueryKeys.login(loginId ?? ''),
    queryFn: async (): Promise<SsoLoginResult> =>
      unwrapResult(await client.pollAwsSsoLogin(loginId ?? '')),
    enabled: !!loginId,
    refetchInterval: q =>
      q.state.data && q.state.data.status !== 'pending' ? false : SSO_POLL_MS,
  })
  const done = query.data?.status === 'succeeded'
  useEffect(() => {
    if (done) {
      void queryClient.invalidateQueries({ queryKey: ['aws', 'credentials'] })
    }
  }, [done, queryClient])
  return query
}
