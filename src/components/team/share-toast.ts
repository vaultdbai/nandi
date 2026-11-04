import { toast } from 'sonner'
import i18n from '@/i18n/config'
import type { ShareResult } from '@/lib/bindings'

/**
 * Says honestly what sharing with a team did: a pushed branch waits for a
 * team admin to merge it (network.md §4.4); a direct push is live
 */
export function toastShared(
  result: ShareResult | null,
  name: string,
  team: string
) {
  const t = i18n.t.bind(i18n)
  const branch = result?.branch
  if (!branch) {
    toast.success(t('teams.shareDialog.done', { name, team }))
    return
  }
  toast.success(t('teams.shareDialog.pushed', { branch }), {
    description: t('teams.shareDialog.pushedHint', { name, team }),
    action: {
      label: t('teams.shareDialog.copyBranch'),
      onClick: () => void navigator.clipboard?.writeText(branch),
    },
  })
}
