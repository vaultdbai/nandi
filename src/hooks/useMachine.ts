/**
 * "Get your computer ready": the core's read-only check of this computer
 * (`check_machine`), polled while a checklist is open, and Start Docker.
 * One query feeds Settings › This computer, the onboarding step, the Home
 * item and the agent page's "Agents need Docker to act".
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { MACHINE_POLL_MS } from '@/components/machine/machine-model'
import { client } from '@/lib/client'
import type { MachineReport } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { plainError, toastError } from '@/lib/toast-error'

export const machineQueryKeys = {
  report: ['machine', 'report'] as const,
}

/** `poll`: re-check every 10 s (a checklist is open) */
export function useMachineReport({ poll = false }: { poll?: boolean } = {}) {
  return useQuery({
    queryKey: machineQueryKeys.report,
    queryFn: async () => unwrapResult(await client.checkMachine(false)),
    refetchInterval: poll ? MACHINE_POLL_MS : false,
    staleTime: 30_000,
    retry: false,
  })
}

/** Check again: re-runs the slower checks too */
export function useCheckMachineAgain() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => unwrapResult(await client.checkMachine(true)),
    onSuccess: (report: MachineReport) =>
      queryClient.setQueryData(machineQueryKeys.report, report),
  })
}

/** Starts Docker Desktop, then the polling notices when it answers */
export function useStartDocker() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => unwrapResult(await client.startDocker()),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: machineQueryKeys.report }),
    onError: error =>
      toastError(error, t('machine.startFailed', { error: plainError(error) })),
  })
}
