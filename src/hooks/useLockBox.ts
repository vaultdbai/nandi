/**
 * Settings › Lock box: the keys by name and rules, and the calls that add,
 * change, delete, unlock and lock them. A key's value goes in once, with
 * `useAddKey`, and never comes back.
 */
import { client } from '@/lib/client'
import type { KeyRules, NewKey } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const lockBoxQueryKeys = {
  all: ['lock-box'] as const,
}

/** Refetched every half minute: last use and unlocks change as agents work */
export function useLockBox() {
  return useClientQuery(lockBoxQueryKeys.all, () => client.getLockBox(), {
    refetchInterval: 30_000,
  })
}

export function useAddKey() {
  return useClientMutation((key: NewKey) => client.addLockBoxKey(key), {
    invalidates: lockBoxQueryKeys.all,
  })
}

export function useUpdateKey() {
  return useClientMutation(
    (input: { name: string; rules: KeyRules }) =>
      client.updateLockBoxKey(input.name, input.rules),
    { invalidates: lockBoxQueryKeys.all }
  )
}

export function useDeleteKey() {
  return useClientMutation((name: string) => client.deleteLockBoxKey(name), {
    invalidates: lockBoxQueryKeys.all,
  })
}

export function useUnlockKey() {
  return useClientMutation((name: string) => client.unlockLockBoxKey(name), {
    invalidates: lockBoxQueryKeys.all,
  })
}

export function useLockKey() {
  return useClientMutation((name: string) => client.lockLockBoxKey(name), {
    invalidates: lockBoxQueryKeys.all,
  })
}
