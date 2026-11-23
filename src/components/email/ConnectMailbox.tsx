import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { PROVIDER_NAMES } from '@/components/settings/email/email-model'
import {
  DEFAULT_READS,
  READ_FAMILIES,
  READ_NAMES,
} from '@/components/settings/email/reads-model'
import { useEmailSignIn } from '@/hooks/useEmail'
import type { MailProvider, Reads } from '@/lib/tauri-bindings'
import { EmailSignInStatus } from './EmailSignInStatus'

/**
 * "Connect Outlook" with Also read my files and Also read my calendar
 * (ticked by default) and the sign-in's progress under it. The one
 * sign-in asks only for mail and what's ticked, read-only.
 */
export function ConnectMailbox({
  provider,
  size = 'sm',
  variant = 'default',
  another = false,
}: {
  provider: MailProvider
  size?: 'sm' | 'default'
  variant?: 'default' | 'outline'
  /** "Add another account": the button stays after one is connected */
  another?: boolean
}) {
  const { t } = useTranslation()
  const id = useId()
  const signIn = useEmailSignIn(provider)
  const [reads, setReads] = useState<Reads>(DEFAULT_READS)
  const waiting = signIn.state.phase === 'waiting'
  const offered = another || signIn.state.phase !== 'connected'
  return (
    <div className="space-y-2" data-connect={provider}>
      {waiting ? (
        <Button size={size} variant="ghost" onClick={signIn.reset}>
          {t('email.cancel')}
        </Button>
      ) : (
        offered && (
          <>
            <Button
              size={size}
              variant={variant}
              onClick={() => void signIn.start(reads)}
            >
              {t(another ? 'email.addAnother' : 'email.connect', {
                name: PROVIDER_NAMES[provider],
              })}
            </Button>
            <div className="space-y-1">
              {READ_FAMILIES.map(family => (
                <label
                  key={family}
                  htmlFor={`${id}-${family}`}
                  className="flex items-center gap-2 text-sm"
                >
                  <Checkbox
                    id={`${id}-${family}`}
                    checked={reads[family]}
                    onCheckedChange={checked =>
                      setReads({ ...reads, [family]: checked === true })
                    }
                  />
                  {t(`email.reads.alsoRead.${family}`, {
                    name: READ_NAMES[provider][family],
                  })}
                </label>
              ))}
            </div>
          </>
        )
      )}
      <EmailSignInStatus state={signIn.state} />
    </div>
  )
}
