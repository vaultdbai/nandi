/**
 * Email (`services::email`): the mailboxes the Chief of Staff reads to keep
 * My notes. The status refreshes on `email://changed` (connect, disconnect,
 * every check) and after a resync.
 */
import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { logger } from '@/lib/logger'
import {
  unwrapResult,
  type BackfillAction,
  type BackfillRange,
  type DeviceFlowState,
  type EmailSignInPoll,
  type EmailSettingsInput,
  type EmailStatus,
  type MailProvider,
  type Result,
  type SignInProblem,
} from '@/lib/tauri-bindings'
import { useDevicePoll } from './useDevicePoll'

export const emailKeys = {
  status: ['email', 'status'] as const,
}

const EMAIL_EVENTS: readonly NandiEventName[] = ['email://changed']

export function useEmailStatus() {
  useInvalidateOn(EMAIL_EVENTS, emailKeys.status)
  return useQuery({
    queryKey: emailKeys.status,
    queryFn: async () => unwrapResult(await client.getEmailStatus()),
  })
}

/** Status-returning actions: Check now, Pause, Disconnect, settings */
export function useEmailAction() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (
      action:
        | { kind: 'check' }
        | { kind: 'pause'; provider: MailProvider; paused: boolean }
        | { kind: 'disconnect'; provider: MailProvider }
        | { kind: 'settings'; settings: EmailSettingsInput }
        | {
            kind: 'backfill'
            provider: MailProvider
            action: BackfillAction
          }
    ): Promise<EmailStatus> => {
      switch (action.kind) {
        case 'check':
          return unwrapResult(await client.checkEmailNow())
        case 'pause':
          return unwrapResult(
            await client.setEmailPaused(action.provider, action.paused)
          )
        case 'disconnect':
          return unwrapResult(await client.disconnectEmail(action.provider))
        case 'settings':
          return unwrapResult(await client.setEmailSettings(action.settings))
        case 'backfill':
          return unwrapResult(
            await client.setEmailBackfill(action.provider, action.action)
          )
      }
    },
    onSuccess: status => queryClient.setQueryData(emailKeys.status, status),
  })
}

/** About how much older mail `range` covers (only asked while connected) */
export function useBackfillEstimate(range: BackfillRange, enabled: boolean) {
  return useQuery({
    queryKey: ['email', 'backfillEstimate', range],
    queryFn: async () =>
      unwrapResult(await client.estimateEmailBackfill(range)),
    enabled: enabled && range !== 'off',
    staleTime: 5 * 60_000,
  })
}

export type EmailSignInState =
  | { phase: 'idle' }
  | { phase: 'waiting'; flow: DeviceFlowState }
  | { phase: 'connected' }
  | { phase: 'failed'; problem: SignInProblem }

const POLL_MS = 1500

/**
 * Connecting one mailbox: `start()` opens the provider's sign-in page in
 * the browser, then the core is polled until the sign-in ends.
 */
export function useEmailSignIn(provider: MailProvider) {
  return useSignInFlow(
    provider,
    () => client.startEmailSignIn(provider),
    () => client.pollEmailSignIn(provider),
    emailKeys.status
  )
}

/**
 * One device or browser sign-in (a mailbox, a calendar, a drive): `start()`
 * opens the sign-in page, then `poll` runs until the sign-in ends and
 * `queryKey` is refetched. `id` names the account; the flow restarts when
 * it changes.
 */
export function useSignInFlow(
  id: string,
  startSignIn: () => Promise<Result<DeviceFlowState, string>>,
  poll: () => Promise<Result<EmailSignInPoll, string>>,
  queryKey: readonly unknown[]
) {
  const queryClient = useQueryClient()
  const [state, setState] = useState<EmailSignInState>({ phase: 'idle' })
  const flow = state.phase === 'waiting' ? state.flow : null
  // The latest poll and key, so the timer doesn't restart on each render
  const latest = useRef({ poll, queryKey })
  useEffect(() => {
    latest.current = { poll, queryKey }
  })

  const intervalMs = Math.max((flow?.interval ?? 0) * 1000, POLL_MS)
  useDevicePoll({
    // A new flow, or another account's, starts a new poll
    session: flow ? `${id}:${flow.deviceCode}` : null,
    delayMs: intervalMs,
    poll: () => latest.current.poll(),
    onPolled: result => {
      if (result.status === 'error') {
        setState({
          phase: 'failed',
          problem: {
            kind: 'other',
            detail: result.error,
            adminConsentUrl: null,
          },
        })
        return null
      }
      const polled = result.data
      if (polled.phase === 'connected') {
        setState({ phase: 'connected' })
        void queryClient.invalidateQueries({
          queryKey: latest.current.queryKey,
        })
        return null
      }
      if (polled.phase === 'failed' && polled.problem) {
        setState({ phase: 'failed', problem: polled.problem })
        return null
      }
      return intervalMs
    },
  })

  const start = async () => {
    const result = await startSignIn()
    if (result.status === 'error') {
      setState({
        phase: 'failed',
        problem: { kind: 'other', detail: result.error, adminConsentUrl: null },
      })
      return
    }
    setState({ phase: 'waiting', flow: result.data })
    client
      .openExternal(result.data.verificationUri)
      .catch((error: unknown) =>
        logger.warn('Could not open the sign-in page', { error })
      )
  }

  const reset = () => setState({ phase: 'idle' })

  return { state, start, reset }
}
