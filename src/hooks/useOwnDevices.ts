/**
 * Settings › Your devices: the paired devices and the calls that pair,
 * confirm, unlock and revoke them. `devices://changed` (a claim, a lock)
 * and `relay://status` refetch the list.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const ownDeviceQueryKeys = {
  all: ['devices'] as const,
}

function useDeviceEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'devices://changed' ||
        event.event === 'relay://status'
      ) {
        void queryClient.invalidateQueries({
          queryKey: ownDeviceQueryKeys.all,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

export function useOwnDevices() {
  useDeviceEvents()
  return useClientQuery(ownDeviceQueryKeys.all, () => client.listOwnDevices())
}

export function useStartPairing() {
  return useClientMutation((name: string) => client.startDevicePairing(name), {
    invalidates: ownDeviceQueryKeys.all,
  })
}

export function useConfirmDevice() {
  return useClientMutation(
    (input: { deviceId: string; pin: string }) =>
      client.confirmOwnDevice(input.deviceId, input.pin),
    { invalidates: ownDeviceQueryKeys.all }
  )
}

export function useRevokeDevice() {
  return useClientMutation(
    (deviceId: string) => client.revokeOwnDevice(deviceId),
    { invalidates: ownDeviceQueryKeys.all }
  )
}

export function useRevokeAllDevices() {
  return useClientMutation(() => client.revokeAllOwnDevices(), {
    invalidates: ownDeviceQueryKeys.all,
  })
}

export function useUnlockDevice() {
  return useClientMutation(
    (deviceId: string) => client.unlockOwnDevice(deviceId, null),
    { invalidates: ownDeviceQueryKeys.all }
  )
}
