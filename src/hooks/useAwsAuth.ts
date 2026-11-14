/**
 * AWS for Bedrock, from the user's own AWS config (TanStack Query): the
 * AWS CLI, profiles, credential status, the Bedrock API key's presence,
 * and SSO login. No hook ever sees key material.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  KeyProfileInput,
  SsoSetupFinish,
  SsoSetupInput,
} from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

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
        event.event === 'aws://credentials_expired'
      ) {
        void queryClient.invalidateQueries({ queryKey: awsQueryKeys.all })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

export function useAwsCli() {
  return useClientQuery(awsQueryKeys.cli, () => client.detectAwsCli(), {
    staleTime: 5 * 60_000,
  })
}

export function useAwsProfiles() {
  useAwsEvents()
  return useClientQuery(awsQueryKeys.profiles, () => client.listAwsProfiles())
}

/** Resolves the profile's credentials and asks STS whose they are */
export function useAwsCredentials(profile: string | null, enabled = true) {
  return useClientQuery(
    awsQueryKeys.credentials(profile),
    () => client.checkAwsCredentials(profile, null),
    { enabled, retry: false }
  )
}

export function useBedrockApiKeyStatus() {
  return useClientQuery(awsQueryKeys.apiKey, () =>
    client.getAwsBedrockApiKeyStatus()
  )
}

/** Starts `aws sso login --profile <profile>` (the browser opens) */
export function useStartSsoLogin() {
  return useClientMutation((profile: string) =>
    client.startAwsSsoLogin(profile)
  )
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
  return useClientMutation(
    (input: KeyProfileInput) => client.saveAwsKeyProfile(input),
    { onSuccess: refresh }
  )
}

/** Starts the IAM Identity Center sign-in for a new SSO profile */
export function useStartSsoSetup() {
  return useClientMutation((input: SsoSetupInput) =>
    client.startAwsSsoSetup(input)
  )
}

/** Polls a setup until it's no longer pending (then it lists accounts) */
export function useSsoSetup(setupId: string | null) {
  return useClientQuery(
    awsQueryKeys.setup(setupId ?? ''),
    () => client.pollAwsSsoSetup(setupId ?? ''),
    {
      enabled: !!setupId,
      retry: false,
      refetchInterval: q =>
        q.state.data && q.state.data.status !== 'pending' ? false : SSO_POLL_MS,
    }
  )
}

/** Writes the SSO profile for the picked account and role */
export function useFinishSsoSetup() {
  const refresh = useRefreshProfiles()
  return useClientMutation(
    (finish: SsoSetupFinish) => client.finishAwsSsoSetup(finish),
    { onSuccess: refresh }
  )
}

/** Polls a login until it's no longer pending; then refreshes credentials */
export function useSsoLogin(loginId: string | null) {
  const queryClient = useQueryClient()
  const query = useClientQuery(
    awsQueryKeys.login(loginId ?? ''),
    () => client.pollAwsSsoLogin(loginId ?? ''),
    {
      enabled: !!loginId,
      refetchInterval: q =>
        q.state.data && q.state.data.status !== 'pending' ? false : SSO_POLL_MS,
    }
  )
  const done = query.data?.status === 'succeeded'
  useEffect(() => {
    if (done) {
      void queryClient.invalidateQueries({ queryKey: ['aws', 'credentials'] })
    }
  }, [done, queryClient])
  return query
}
