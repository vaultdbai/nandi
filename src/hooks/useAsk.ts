/**
 * The ask box's state: sends an ask with the view's context and holds the
 * latest turn (pending, streaming, answered, or failed) until it's
 * dismissed or the next ask replaces it. Answers stream in
 * (`ask://streaming_delta`, then `ask://completed`); finished asks land in
 * the history (`useAskHistory`).
 */
import { useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { FOLLOWUPS_MARKER } from '@/lib/ask'
import type { ActionPlan, AskContext, AskResponse } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export interface AskTurn {
  /** Client-side ID; a newer ask replaces an older pending one */
  id: string
  text: string
  status: 'pending' | 'done' | 'error'
  /** The answer so far, while it streams (follow-ups left out) */
  streamed: string
  response: AskResponse | null
  error: string | null
}

export const askKeys = {
  all: ['ask'] as const,
  history: (limit: number) => ['ask', 'history', limit] as const,
}

interface StreamState {
  deltas: string
  completed: { response: AskResponse | null; error: string | null } | null
}

/** Event streams kept for request IDs (theirs can beat the reply) */
const STREAMS_KEPT = 20

/** The answer as it streams, without the follow-ups line */
export function visibleAnswer(text: string): string {
  const at = text.indexOf(FOLLOWUPS_MARKER)
  return (at === -1 ? text : text.slice(0, at)).trimEnd()
}

export function useAsk(
  context: AskContext,
  /** Called once when an ask comes back as a plan; `dismiss` clears it */
  onAction?: (plan: ActionPlan, text: string, dismiss: () => void) => void
) {
  const queryClient = useQueryClient()
  const [turn, setTurn] = useState<AskTurn | null>(null)
  // Request ID → what arrived; the turn the current request belongs to
  const streams = useRef(new Map<string, StreamState>())
  const current = useRef<{
    turnId: string
    requestId: string
    text: string
  } | null>(null)
  const onActionRef = useRef(onAction)
  useEffect(() => {
    onActionRef.current = onAction
  })

  // Reads refs only, so the subscription below can call it
  const applyRef = useRef((requestId: string) => {
    const owner = current.current
    const stream = streams.current.get(requestId)
    if (owner?.requestId !== requestId || !stream) return
    const completed = stream.completed
    setTurn(previous => {
      if (previous?.id !== owner.turnId) return previous
      if (!completed)
        return { ...previous, streamed: visibleAnswer(stream.deltas) }
      return completed.response
        ? { ...previous, status: 'done', response: completed.response }
        : {
            ...previous,
            status: 'error',
            error: completed.error ?? 'The ask failed',
          }
    })
    if (completed) {
      streams.current.delete(requestId)
      void queryClient.invalidateQueries({ queryKey: askKeys.all })
      if (completed.response?.kind === 'action') {
        const plan = completed.response.data
        onActionRef.current?.(plan, owner.text, () => {
          if (current.current?.turnId !== owner.turnId) return
          current.current = null
          setTurn(null)
        })
      }
    }
  })

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event !== 'ask://streaming_delta' &&
        event.event !== 'ask://completed'
      ) {
        return
      }
      const { requestId } = event.payload
      const stream = streams.current.get(requestId) ?? {
        deltas: '',
        completed: null,
      }
      if (event.event === 'ask://streaming_delta') {
        stream.deltas += event.payload.delta
      } else {
        stream.completed = {
          response: event.payload.response,
          error: event.payload.error,
        }
      }
      streams.current.set(requestId, stream)
      // Forget other boxes' and old requests
      while (streams.current.size > STREAMS_KEPT) {
        const oldest = streams.current.keys().next().value
        if (oldest === undefined) break
        streams.current.delete(oldest)
      }
      applyRef.current(requestId)
    })
    return subscription.unsubscribe
  }, [])

  const submit = async (text: string, attachments: string[] = []) => {
    const id = crypto.randomUUID()
    current.current = null
    setTurn({
      id,
      text,
      status: 'pending',
      streamed: '',
      response: null,
      error: null,
    })
    const result = await client.askStreaming({ text, attachments, context })
    if (result.status === 'error') {
      setTurn(previous =>
        previous?.id === id
          ? { ...previous, status: 'error', error: result.error }
          : previous
      )
      return
    }
    current.current = { turnId: id, requestId: result.data, text }
    // Anything that arrived before the request ID did
    applyRef.current(result.data)
  }

  return {
    turn,
    pending: turn?.status === 'pending',
    submit,
    dismiss: () => {
      current.current = null
      setTurn(null)
    },
  }
}

/** Recent asks, newest first */
export function useAskHistory(limit = 20) {
  return useQuery({
    queryKey: askKeys.history(limit),
    queryFn: async () => unwrapResult(await client.listAskHistory(limit)),
  })
}
