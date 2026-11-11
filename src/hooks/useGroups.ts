/**
 * Network › Groups: the relay's groups, one group in full, and the calls
 * that change them. Under `['network', 'groups']`, so `network://*` events
 * (`network://group_changed` among them, see `useMemberEvents`) refetch.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  AccessLevel,
  GroupRole,
  GroupView,
  InviteRequest,
} from '@/lib/bindings'
import { useMemberEvents } from './useNetwork'
import { unwrapResult } from '@/lib/tauri-bindings'

export const groupQueryKeys = {
  all: ['network', 'groups'] as const,
  list: ['network', 'groups', 'list'] as const,
  one: (groupId: string) => ['network', 'groups', 'one', groupId] as const,
}

export function useGroups() {
  useMemberEvents()
  return useQuery({
    queryKey: groupQueryKeys.list,
    queryFn: () => client.listGroups().then(unwrapResult),
  })
}

export function useGroup(groupId: string) {
  useMemberEvents()
  return useQuery({
    queryKey: groupQueryKeys.one(groupId),
    queryFn: () => client.getGroup(groupId).then(unwrapResult),
    retry: false,
  })
}

/** A change that answers with the group: it replaces the cache, the list refetches */
function useGroupMutation<A>(call: (args: A) => Promise<GroupView>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: call,
    onSuccess: view => {
      queryClient.setQueryData(groupQueryKeys.one(view.group.summary.id), view)
      void queryClient.invalidateQueries({ queryKey: groupQueryKeys.list })
    },
  })
}

/** A change that doesn't: everything under groups refetches */
function useGroupsChange<A, T>(call: (args: A) => Promise<T>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: call,
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: groupQueryKeys.all }),
  })
}

export function useCreateGroup() {
  return useGroupMutation(
    (input: { name: string; description: string; policy: string }) =>
      client
        .createGroup(input.name, input.description, input.policy)
        .then(unwrapResult)
  )
}

export function useUpdateGroup() {
  return useGroupMutation(
    (input: { groupId: string; name: string; description: string }) =>
      client
        .updateGroup(input.groupId, input.name, input.description)
        .then(unwrapResult)
  )
}

export function useSetGroupPolicy() {
  return useGroupMutation((input: { groupId: string; policy: string }) =>
    client.setGroupPolicy(input.groupId, input.policy).then(unwrapResult)
  )
}

export function useJoinGroup() {
  return useGroupMutation((code: string) =>
    client.joinGroup(code).then(unwrapResult)
  )
}

export function useRemoveGroupMember() {
  return useGroupMutation((input: { groupId: string; instanceId: string }) =>
    client.removeGroupMember(input.groupId, input.instanceId).then(unwrapResult)
  )
}

export function useSetGroupRole() {
  return useGroupMutation(
    (input: { groupId: string; instanceId: string; role: GroupRole }) =>
      client
        .setGroupRole(input.groupId, input.instanceId, input.role)
        .then(unwrapResult)
  )
}

export function useInviteToGroup() {
  return useGroupsChange((input: { groupId: string; invite: InviteRequest }) =>
    client.inviteToGroup(input.groupId, input.invite).then(unwrapResult)
  )
}

export function useRevokeGroupInvite() {
  return useGroupsChange((code: string) =>
    client.revokeGroupInvite(code).then(unwrapResult)
  )
}

export function useLeaveGroup() {
  return useGroupsChange((groupId: string) =>
    client.leaveGroup(groupId).then(unwrapResult)
  )
}

export function useSetGroupAccess() {
  return useGroupsChange((input: { groupId: string; level: AccessLevel }) =>
    client.setGroupAccess(input.groupId, input.level).then(unwrapResult)
  )
}
