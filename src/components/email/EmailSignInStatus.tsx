import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { problemKey } from '@/components/settings/email/email-model'
import type { EmailSignInState } from '@/hooks/useEmail'
import { client } from '@/lib/client'

/**
 * Where connecting a mailbox is: waiting for the browser (or a device
 * code), connected, or why it didn't work (an admin's approval comes with
 * the link to forward to IT)
 */
export function EmailSignInStatus({ state }: { state: EmailSignInState }) {
  const { t } = useTranslation()
  if (state.phase === 'waiting') {
    return (
      <div className="space-y-1 rounded-md border p-3 text-sm">
        {state.flow.userCode ? (
          <>
            <p>{t('email.enterCode')}</p>
            <p className="font-mono text-lg tracking-widest" dir="ltr">
              {state.flow.userCode}
            </p>
          </>
        ) : (
          <p>{t('email.finishInBrowser')}</p>
        )}
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Spinner />
          {t('email.waiting')}
        </p>
      </div>
    )
  }
  if (state.phase === 'connected') {
    return <p className="text-sm text-success">{t('email.connected')}</p>
  }
  if (state.phase === 'failed') {
    const { problem } = state
    const link = problem.adminConsentUrl
    return (
      <div className="space-y-2 rounded-md border p-3 text-sm">
        <p>{t(problemKey(problem.kind))}</p>
        {link && (
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => void navigator.clipboard?.writeText(link)}
            >
              {t('email.copyAdminLink')}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void client.openExternal(link)}
            >
              {t('email.openAdminLink')}
            </Button>
          </div>
        )}
        <p className="text-xs break-words text-muted-foreground">
          {problem.detail}
        </p>
      </div>
    )
  }
  return null
}
