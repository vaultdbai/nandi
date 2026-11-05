import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { PROVIDER_NAMES } from '@/components/settings/email/email-model'
import { useEmailSignIn } from '@/hooks/useEmail'
import type { MailProvider } from '@/lib/tauri-bindings'
import { EmailSignInStatus } from './EmailSignInStatus'

/** "Connect Outlook" with the sign-in's progress under it */
export function ConnectMailbox({
  provider,
  size = 'sm',
  variant = 'default',
}: {
  provider: MailProvider
  size?: 'sm' | 'default'
  variant?: 'default' | 'outline'
}) {
  const { t } = useTranslation()
  const signIn = useEmailSignIn(provider)
  const waiting = signIn.state.phase === 'waiting'
  return (
    <div className="space-y-2" data-connect={provider}>
      {waiting ? (
        <Button size={size} variant="ghost" onClick={signIn.reset}>
          {t('email.cancel')}
        </Button>
      ) : (
        signIn.state.phase !== 'connected' && (
          <Button
            size={size}
            variant={variant}
            onClick={() => void signIn.start()}
          >
            {t('email.connect', { name: PROVIDER_NAMES[provider] })}
          </Button>
        )
      )}
      <EmailSignInStatus state={signIn.state} />
    </div>
  )
}
