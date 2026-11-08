/**
 * The Chief of Staff's auto-approve rules: what lets a screened item through
 * without a click. Read in Settings; "Always allow" on a queued item adds one.
 */
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import type { QueueRow } from '@/components/home/digest-model'
import {
  addRule,
  checkNewRule,
  domainShortcut,
} from '@/components/settings/auto-approve/auto-approve-model'
import { client } from '@/lib/client'
import type { AutoApproveRule } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const autoApproveKeys = {
  all: ['chief-of-staff', 'auto-approve'] as const,
}

export function useAutoApproveRules() {
  return useQuery({
    queryKey: autoApproveKeys.all,
    queryFn: async () => unwrapResult(await client.getAutoApproveRules()),
  })
}

/** Replaces the whole list; the core's answer is the error when a rule is wrong */
export function useSaveAutoApproveRules() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (rules: AutoApproveRule[]) => {
      unwrapResult(await client.setAutoApproveRules(rules))
      return rules
    },
    onSuccess: rules => queryClient.setQueryData(autoApproveKeys.all, rules),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: autoApproveKeys.all }),
  })
}

/** Adds one rule to the list as the core has it now (not a possibly stale copy) */
export function useAddAutoApproveRule() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (rule: AutoApproveRule) => {
      const current = unwrapResult(await client.getAutoApproveRules())
      const problem = checkNewRule(current, rule)
      if (problem) throw new Error(problem)
      const rules = addRule(current, rule)
      unwrapResult(await client.setAutoApproveRules(rules))
      return rules
    },
    onSuccess: rules => queryClient.setQueryData(autoApproveKeys.all, rules),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: autoApproveKeys.all }),
  })
}

/** A local check's i18n key becomes its sentence; the core's text stays */
function problemText(error: unknown, t: (key: string) => string): string {
  const text = error instanceof Error ? error.message : String(error)
  return text.startsWith('autoApprove.error.') ? t(text) : text
}

/**
 * "Always allow mail from <domain>" on a queued email item: asks first, then
 * adds a domain rule with the item's action. Shared by Home and the thread
 * page, like `useQueueDecisions`.
 */
export function useAlwaysAllow() {
  const { t } = useTranslation()
  const add = useAddAutoApproveRule()
  const [asking, setAsking] = useState<QueueRow | null>(null)

  const confirm = () => {
    const row = asking
    setAsking(null)
    if (!row?.alwaysAllowDomain) return
    add.mutate(domainShortcut(row.alwaysAllowDomain, row.action), {
      onSuccess: () =>
        toast.success(
          t('autoApprove.shortcut.added', { domain: row.alwaysAllowDomain })
        ),
      onError: error =>
        toast.error(t('autoApprove.shortcut.failed'), {
          description: problemText(error, t),
        }),
    })
  }

  return {
    busy: add.isPending,
    asking,
    ask: (row: QueueRow) => setAsking(row),
    cancel: () => setAsking(null),
    confirm,
  }
}
