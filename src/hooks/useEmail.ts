/**
 * Email (`services::email`): the mailboxes Nandi reads to keep
 * My notes. The status refreshes on `email://changed` (connect, disconnect,
 * every check) and after a resync.
 */
import { useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'
import { logger } from '@/lib/logger'
import type {
  BackfillAction,
  BackfillRange,
  DeviceFlowState,
  EmailSignInPoll,
  EmailSettingsInput,
  MailboxId,
  MailProvider,
  Reads,
  Result,
  SignInProblem,
} from '@/lib/tauri-bindings'
import { useDevicePoll } from './useDevicePoll'

export const emailKeys = {
  status: ['email', 'status'] as const,
}

const EMAIL_EVENTS: readonly NandiEventName[] = ['email://changed']

export function useEmailStatus() {
  useInvalidateOn(EMAIL_EVENTS, emailKeys.status)
  return useClientQuery(emailKeys.status, () => client.getEmailStatus())
}

/** Status-returning actions: Check now, Pause, Disconnect, settings */
export function useEmailAction() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (
      action:
        | { kind: 'check' }
        | { kind: 'again' }
        | { kind: 'pause'; mailbox: MailboxId; paused: boolean }
        | { kind: 'disconnect'; mailbox: MailboxId }
        | { kind: 'reads'; mailbox: MailboxId; reads: Reads }
        | { kind: 'settings'; settings: EmailSettingsInput }
        | {
            kind: 'backfill'
            mailbox: MailboxId
            action: BackfillAction
          }
    ) => {
      switch (action.kind) {
        case 'check':
          return client.checkEmailNow()
        case 'again':
          return client.readEmailAgain()
        case 'pause':
          return client.setEmailPaused(action.mailbox, action.paused)
        case 'disconnect':
          return client.disconnectEmail(action.mailbox)
        case 'reads':
          return client.setEmailReads(action.mailbox, action.reads)
        case 'settings':
          return client.setEmailSettings(action.settings)
        case 'backfill':
          return client.setEmailBackfill(action.mailbox, action.action)
      }
    },
    {
      onSuccess: status => queryClient.setQueryData(emailKeys.status, status),
    }
  )
}

/** About how much older mail `range` covers (only asked while connected) */
export function useBackfillEstimate(range: BackfillRange, enabled: boolean) {
  return useClientQuery(
    ['email', 'backfillEstimate', range],
    () => client.estimateEmailBackfill(range),
    { enabled: enabled && range !== 'off', staleTime: 5 * 60_000 }
  )
}

export type EmailSignInState =
  | { phase: 'idle' }
  | { phase: 'waiting'; flow: DeviceFlowState }
  | { phase: 'connected' }
  | { phase: 'failed'; problem: SignInProblem }

const POLL_MS = 1500

/**
 * Connecting one account: `start(reads)` opens the provider's sign-in page
 * in the browser, asking for what `reads` ticks (mail, files, calendar),
 * then the core is polled until the sign-in ends.
 */
export function useEmailSignIn(provider: MailProvider) {
  return useSignInFlow(
    provider,
    (reads: Reads) => client.startEmailSignIn(provider, reads),
    () => client.pollEmailSignIn(provider),
    emailKeys.status
  )
}

/**
 * One device or browser sign-in: `start(reads)` opens the sign-in page,
 * then `poll` runs until the sign-in ends and `queryKey` is refetched.
 * `id` names the provider; the flow restarts when it changes.
 */
function useSignInFlow(
  id: string,
  startSignIn: (reads: Reads) => Promise<Result<DeviceFlowState, string>>,
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

  const start = async (reads: Reads) => {
    const result = await startSignIn(reads)
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
