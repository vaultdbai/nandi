/**
 * "Get your computer ready": the core's read-only check of this computer
 * (`check_machine`), polled while a checklist is open, and Start Docker.
 * One query feeds Settings › This computer, the onboarding step, the Home
 * item and the agent page's "Agents need Docker to act". Phase 2: the one
 * go-ahead (`get_computer_ready`, live on `machine://ready`) and "Start
 * Docker when Nandi opens" (a portable setting).
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { MACHINE_POLL_MS } from '@/components/machine/machine-model'
import { client } from '@/lib/client'
import type { MachineReport, ReadyProgress, ReadyStep } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'
import { plainError, toastError } from '@/lib/toast-error'

export const machineQueryKeys = {
  report: ['machine', 'report'] as const,
  ready: ['machine', 'ready'] as const,
  settings: ['machine', 'settings'] as const,
}

/** `poll`: re-check every 10 s (a checklist is open) */
export function useMachineReport({ poll = false }: { poll?: boolean } = {}) {
  return useClientQuery(
    machineQueryKeys.report,
    () => client.checkMachine(false),
    {
      refetchInterval: poll ? MACHINE_POLL_MS : false,
      staleTime: 30_000,
      retry: false,
    }
  )
}

/** Check again: re-runs the slower checks too */
export function useCheckMachineAgain() {
  const queryClient = useQueryClient()
  return useClientMutation(() => client.checkMachine(true), {
    onSuccess: (report: MachineReport) =>
      queryClient.setQueryData(machineQueryKeys.report, report),
  })
}

/** Starts Docker Desktop, then the polling notices when it answers */
export function useStartDocker() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  return useClientMutation(() => client.startDocker(), {
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: machineQueryKeys.report }),
    onError: error =>
      toastError(error, t('machine.startFailed', { error: plainError(error) })),
  })
}

/** How "Get your computer ready" is going, kept live by `machine://ready` */
export function useComputerReady() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event !== 'machine://ready') return
      const progress: ReadyProgress = event.payload
      queryClient.setQueryData(machineQueryKeys.ready, progress)
      if (progress.report)
        queryClient.setQueryData(machineQueryKeys.report, progress.report)
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
  return useClientQuery(machineQueryKeys.ready, () =>
    client.computerReadyStatus()
  )
}

/** Go ahead: runs the steps the user saw, in the background */
export function useGetComputerReady() {
  const { t } = useTranslation()
  return useClientMutation(
    (steps: ReadyStep[]) => client.getComputerReady(steps),
    {
      invalidates: machineQueryKeys.ready,
      onError: error =>
        toastError(
          error,
          t('machine.ready.startFailed', { error: plainError(error) })
        ),
    }
  )
}

export function useMachineSettings() {
  return useClientQuery(machineQueryKeys.settings, () =>
    client.machineSettings()
  )
}

/** The answer to "Start Docker automatically when Nandi opens?" */
export function useSetStartDockerOnOpen() {
  return useClientMutation(
    (start: boolean) => client.setStartDockerOnOpen(start),
    { invalidates: machineQueryKeys.settings }
  )
}
