/**
 * A run's cached summary and question answers (TanStack Query).
 *
 * The backend caches both in `local_settings` (finished runs only), so the
 * summary query never goes stale here and re-asking a question is cheap.
 * Answers asked this session stay in the query cache for the Analysis tab.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { AnalysisResponse } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const runAnalysisKeys = {
  summary: (runId: string) => ['runAnalysis', runId, 'summary'] as const,
  answers: (runId: string) => ['runAnalysis', runId, 'answers'] as const,
}

/** Errors arrive as strings from Rust; keep them readable */
export function analysisErrorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  // A rejected Result (`{ status: 'error', error }`)
  if (error && typeof error === 'object' && 'error' in error) {
    return analysisErrorMessage((error as { error: unknown }).error)
  }
  return String(error)
}

export function useRunAnalysis(runId: string, { enabled = true } = {}) {
  const queryClient = useQueryClient()

  const summary = useQuery({
    queryKey: runAnalysisKeys.summary(runId),
    queryFn: async () => unwrapResult(await client.summarizeRun(runId)),
    enabled,
    staleTime: Infinity,
    retry: false,
  })

  // Client-side list: filled by `ask`, never fetched
  const answers = useQuery({
    queryKey: runAnalysisKeys.answers(runId),
    queryFn: () =>
      queryClient.getQueryData<AnalysisResponse[]>(
        runAnalysisKeys.answers(runId)
      ) ?? [],
    initialData: [] as AnalysisResponse[],
    staleTime: Infinity,
    gcTime: Infinity,
  })

  const ask = useMutation({
    mutationFn: async (question: string) =>
      unwrapResult(await client.analyzeRun(runId, question)),
    onSuccess: response => {
      queryClient.setQueryData<AnalysisResponse[]>(
        runAnalysisKeys.answers(runId),
        (previous = []) => [
          ...previous.filter(a => a.question !== response.question),
          response,
        ]
      )
    },
  })

  return { summary, answers: answers.data, ask }
}
