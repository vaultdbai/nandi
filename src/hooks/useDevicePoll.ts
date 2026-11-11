import { useEffect, useRef } from 'react'

interface DevicePoll<S extends object | string, T> {
  /**
   * The sign-in being polled, or `null` while there is none. Polling starts
   * when it appears and stops, even mid-request, when it changes (by
   * identity, so keep it stable) or goes back to `null`.
   */
  session: S | null
  /** The wait before the first poll, in ms */
  delayMs: number
  /** Asks the provider once about this session */
  poll: (session: S) => Promise<T>
  /**
   * Applies one answer. Returns the wait before the next poll in ms (the
   * provider may ask for a longer one), or `null` when the sign-in ended.
   * Never called for a session that has been cancelled.
   */
  onPolled: (answer: T) => number | null
}

/**
 * Polls a device or browser sign-in until it ends. The next poll waits for
 * the last one to finish, so a slow provider is never asked twice at once,
 * and a poll that returns after Cancel is dropped instead of reviving the
 * sign-in.
 */
export function useDevicePoll<S extends object | string, T>({
  session,
  delayMs,
  poll,
  onPolled,
}: DevicePoll<S, T>) {
  // The latest callbacks, so a re-render doesn't restart the timer
  const latest = useRef({ delayMs, poll, onPolled })
  useEffect(() => {
    latest.current = { delayMs, poll, onPolled }
  })

  useEffect(() => {
    if (session === null) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined

    const schedule = (waitMs: number) => {
      timer = setTimeout(() => void tick(), waitMs)
    }
    const tick = async () => {
      const answer = await latest.current.poll(session)
      if (cancelled) return
      const nextMs = latest.current.onPolled(answer)
      if (nextMs !== null) schedule(nextMs)
    }

    schedule(latest.current.delayMs)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [session])
}
