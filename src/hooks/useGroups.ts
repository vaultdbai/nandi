/**
 * Network › Groups: the relay's groups, one group in full, and the calls
 * that change them. Under `['network', 'groups']`, so `network://*` events
 * (`network://group_changed` among them, see `useMemberEvents`) refetch.
 */
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  AccessLevel,
  GroupRole,
  GroupView,
  InviteRequest,
} from '@/lib/bindings'
import { useMemberEvents } from './useNetwork'
import type { Result } from '@/lib/tauri-bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const groupQueryKeys = {
  all: ['network', 'groups'] as const,
  list: ['network', 'groups', 'list'] as const,
  one: (groupId: string) => ['network', 'groups', 'one', groupId] as const,
}

export function useGroups() {
  useMemberEvents()
  return useClientQuery(groupQueryKeys.list, () => client.listGroups())
}

export function useGroup(groupId: string) {
  useMemberEvents()
  return useClientQuery(
    groupQueryKeys.one(groupId),
    () => client.getGroup(groupId),
    { retry: false }
  )
}

/** A change that answers with the group: it replaces the cache, the list refetches */
function useGroupMutation<A>(
  call: (args: A) => Promise<Result<GroupView, string>>
) {
  const queryClient = useQueryClient()
  return useClientMutation(call, {
    onSuccess: view => {
      queryClient.setQueryData(groupQueryKeys.one(view.group.summary.id), view)
      void queryClient.invalidateQueries({ queryKey: groupQueryKeys.list })
    },
  })
}

/** A change that doesn't: everything under groups refetches */
function useGroupsChange<A, T>(call: (args: A) => Promise<Result<T, string>>) {
  const queryClient = useQueryClient()
  return useClientMutation(call, {
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: groupQueryKeys.all }),
  })
}

export function useCreateGroup() {
  return useGroupMutation(
    (input: { name: string; description: string; policy: string }) =>
      client.createGroup(input.name, input.description, input.policy)
  )
}

export function useSetGroupPolicy() {
  return useGroupMutation((input: { groupId: string; policy: string }) =>
    client.setGroupPolicy(input.groupId, input.policy)
  )
}

export function useJoinGroup() {
  return useGroupMutation((code: string) => client.joinGroup(code))
}

export function useRemoveGroupMember() {
  return useGroupMutation((input: { groupId: string; instanceId: string }) =>
    client.removeGroupMember(input.groupId, input.instanceId)
  )
}

export function useSetGroupRole() {
  return useGroupMutation(
    (input: { groupId: string; instanceId: string; role: GroupRole }) =>
      client.setGroupRole(input.groupId, input.instanceId, input.role)
  )
}

export function useInviteToGroup() {
  return useGroupsChange((input: { groupId: string; invite: InviteRequest }) =>
    client.inviteToGroup(input.groupId, input.invite)
  )
}

export function useRevokeGroupInvite() {
  return useGroupsChange((code: string) => client.revokeGroupInvite(code))
}

export function useLeaveGroup() {
  return useGroupsChange((groupId: string) => client.leaveGroup(groupId))
}

export function useSetGroupAccess() {
  return useGroupsChange((input: { groupId: string; level: AccessLevel }) =>
    client.setGroupAccess(input.groupId, input.level)
  )
}
