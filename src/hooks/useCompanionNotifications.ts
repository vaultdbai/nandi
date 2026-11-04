/**
 * OS notifications from the companion (companion.md §5): approvals and
 * questions that appear while it's hidden or collapsed, and items 5
 * minutes from expiring. On by default ("Notify me"). The rules live in
 * `noticesFor`; this hook only remembers what it has seen and sends.
 *
 * The desktop notification plugin has no action buttons, so a notification
 * can't open the card at its item directly; the item is at the top of
 * Needs you whenever the companion opens.
 */
import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { useNow } from '@/components/agents/task-utils'
import {
  noticesFor,
  type Notice,
  type NoticeState,
} from '@/components/companion/companion-model'
import type { InboxItem } from '@/components/inbox/inbox-model'
import { isDesktop } from '@/lib/client'
import { logger } from '@/lib/logger'
import { commands } from '@/lib/tauri-bindings'

function itemLine(
  item: InboxItem,
  t: ReturnType<typeof useTranslation>['t']
): string {
  switch (item.kind) {
    case 'approval':
      return t('companion.notice.approval', { agent: item.agentName })
    case 'workflowApproval':
      return t('companion.notice.workflowApproval', { name: item.item.title })
    case 'gatewayHold':
      return t('companion.notice.hold', { member: item.hold.callerName })
    case 'clarification':
      return t('companion.notice.question', {
        question: item.clarification.question.question,
      })
    case 'requestReturn':
      return t(`inbox.return.${item.ret.outcome}`, { name: item.ret.fromName })
    default:
      return t('companion.notice.generic')
  }
}

export function useCompanionNotifications({
  items,
  away,
  enabled,
  results,
}: {
  items: InboxItem[]
  /** The companion is hidden or collapsed to the orb */
  away: boolean
  enabled: boolean
  /** "Notify me when requests come back" */
  results: boolean
}) {
  const { t } = useTranslation()
  // Deadlines come close without an event
  const now = useNow(true, 30_000)
  const state = useRef<NoticeState | null>(null)

  useEffect(() => {
    const result = noticesFor({
      items,
      state: state.current,
      now,
      away,
      enabled,
      results,
    })
    state.current = result.state
    if (!isDesktop()) return
    for (const notice of result.notices) void send(notice)

    async function send(notice: Notice) {
      const line = itemLine(notice.item, t)
      const title =
        notice.kind === 'expiring' ? t('companion.notice.expiring') : line
      const body =
        notice.kind === 'expiring'
          ? line
          : notice.kind === 'result' && notice.item.kind === 'requestReturn'
            ? notice.item.ret.summary || t('companion.notice.openHint')
            : t('companion.notice.openHint')
      const sent = await commands.sendNativeNotification(title, body)
      if (sent.status === 'error') {
        logger.warn('Companion: notification failed', { error: sent.error })
      }
    }
  }, [items, now, away, enabled, results, t])
}
