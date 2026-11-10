import { useTranslation } from 'react-i18next'
import { Copy } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { GithubSignInPanel } from '@/components/github/GithubSignInPanel'
import { useRelay } from '@/hooks/useRelay'
import { useNavigationStore } from '@/store/navigation-store'
import { IdentitySection } from '../IdentitySection'
import { AdvancedBlock, SettingsSectionFrame } from '../SettingsFrame'

/** Settings › Account: how you appear to others; the device under Details */
export function AccountSection() {
  const { t } = useTranslation()
  const { status } = useRelay()
  const deviceId = status?.instanceId ?? ''

  return (
    <SettingsSectionFrame
      title={t('settings.section.account')}
      description={t('settings.account.description')}
    >
      <IdentitySection />
      <section className="space-y-2" aria-labelledby="account-github">
        <h3 id="account-github" className="text-sm font-medium">
          {t('github.signIn.title')}
        </h3>
        <p className="text-sm text-muted-foreground">
          {t('github.signIn.description')}
        </p>
        <GithubSignInPanel />
      </section>
      <section className="space-y-2" aria-labelledby="account-setup-again">
        <h3 id="account-setup-again" className="text-sm font-medium">
          {t('settings.account.setupAgain.title')}
        </h3>
        <p className="text-sm text-muted-foreground">
          {t('settings.account.setupAgain.description')}
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'onboarding', again: true })
          }
        >
          {t('settings.account.setupAgain.button')}
        </Button>
      </section>
      <AdvancedBlock label={t('settings.details')}>
        <dl className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-sm">
          <dt className="text-muted-foreground">
            {t('settings.account.device')}
          </dt>
          <dd className="flex min-w-0 items-center gap-2">
            <code dir="ltr" className="truncate font-mono text-xs">
              {deviceId || '—'}
            </code>
            {deviceId && (
              <Button
                size="sm"
                variant="ghost"
                aria-label={t('settings.account.copyDevice')}
                onClick={() => void navigator.clipboard?.writeText(deviceId)}
              >
                <Copy />
              </Button>
            )}
          </dd>
          <dt className="text-muted-foreground">
            {t('settings.account.file')}
          </dt>
          <dd>
            <code dir="ltr" className="font-mono text-xs">
              ~/.nandi/repo/NANDI.md
            </code>
          </dd>
        </dl>
      </AdvancedBlock>
    </SettingsSectionFrame>
  )
}
