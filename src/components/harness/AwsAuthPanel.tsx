import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { KeyRound, LogIn, Plus, RefreshCw } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { errorMessage } from '@/hooks/useDelegation'
import {
  useAwsCli,
  useAwsCredentials,
  useAwsProfiles,
  useBedrockApiKeyStatus,
  useStartSsoLogin,
} from '@/hooks/useAwsAuth'
import { AwsKeysDialog } from './AwsKeysDialog'
import { AwsSsoSetupDialog } from './AwsSsoSetupDialog'
import { maskedIdentity } from './aws-identity'
import { canLogIn } from './aws-setup-model'
import { SsoLoginDialog } from './SsoLoginDialog'

/**
 * AWS as the user has it set up: the AWS CLI, profiles from the user's AWS
 * config, whether the chosen profile's credentials work (and whose they
 * are), a Bedrock API key in the environment, and signing in the way the
 * profile is set up (SSO, `aws login`, or new keys). New profiles (SSO or
 * access keys) are written by the AWS CLI. Nandi stores none of it.
 */
export function AwsAuthPanel() {
  const { t } = useTranslation()
  const cli = useAwsCli()
  const profiles = useAwsProfiles()
  const apiKey = useBedrockApiKeyStatus()
  const [chosen, setChosen] = useState<string | null>(null)
  const [loginFor, setLoginFor] = useState<string | null>(null)
  const [loginId, setLoginId] = useState<string | null>(null)
  const [showArn, setShowArn] = useState(false)
  // `''`: a new access-key profile; a name: new keys for it
  const [keysFor, setKeysFor] = useState<string | null>(null)
  const [addingSso, setAddingSso] = useState(false)
  const startLogin = useStartSsoLogin()

  const logIn = (profile: string) => {
    setLoginFor(profile)
    setLoginId(null)
    startLogin.mutate(profile, {
      onSuccess: result => setLoginId(result.loginId),
    })
  }

  const all = profiles.data ?? []
  const fallback = all.find(p => p.isDefault)?.name ?? all[0]?.name ?? null
  const selected = all.find(p => p.name === (chosen ?? fallback)) ?? null
  const credentials = useAwsCredentials(selected?.name ?? null, !!selected)
  const status = credentials.data

  return (
    <div className="flex flex-col gap-3" data-testid="aws-auth">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Badge variant={cli.data?.installed ? 'secondary' : 'outline'}>
          {cli.data?.installed
            ? t('harness.aws.cli', { version: cli.data.version ?? '?' })
            : t('harness.aws.noCli')}
        </Badge>
        {apiKey.data?.configured && (
          <Badge variant="secondary">
            <KeyRound />
            {t('harness.aws.apiKey', {
              source: t(
                `harness.aws.source.${apiKey.data.source ?? 'environment'}`
              ),
            })}
          </Badge>
        )}
      </div>

      {profiles.isPending ? (
        <Skeleton className="h-9 w-full" />
      ) : all.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('harness.aws.noProfilesYet')}
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <NativeSelect
            aria-label={t('harness.aws.profile')}
            value={selected?.name ?? ''}
            onChange={event => setChosen(event.target.value)}
          >
            {all.map(profile => (
              <NativeSelectOption key={profile.name} value={profile.name}>
                {profile.name}
                {profile.region ? ` (${profile.region})` : ''}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          <Button
            size="sm"
            variant="outline"
            disabled={credentials.isFetching}
            onClick={() => void credentials.refetch()}
          >
            {credentials.isFetching ? <Spinner /> : <RefreshCw />}
            {t('harness.aws.check')}
          </Button>
          {selected && canLogIn(selected, cli.data) && (
            <Button
              size="sm"
              disabled={startLogin.isPending}
              onClick={() => logIn(selected.name)}
            >
              <LogIn />
              {t('harness.aws.login')}
            </Button>
          )}
          {selected?.kind === 'staticKeys' && (
            <Button
              size="sm"
              variant="outline"
              disabled={!cli.data?.installed}
              onClick={() => setKeysFor(selected.name)}
            >
              <KeyRound />
              {t('harness.aws.keys.replace')}
            </Button>
          )}
        </div>
      )}
      {selected?.kind === 'consoleLogin' && !cli.data?.supportsLogin && (
        <p className="text-xs text-muted-foreground">
          {t('harness.aws.consoleLoginNeedsCli')}
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={!cli.data?.supportsSso}
          onClick={() => setAddingSso(true)}
        >
          <Plus />
          {t('harness.aws.setup.addSso')}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!cli.data?.installed}
          onClick={() => setKeysFor('')}
        >
          <Plus />
          {t('harness.aws.setup.addKeys')}
        </Button>
        {cli.data && !cli.data.installed && (
          <span className="text-xs text-muted-foreground">
            {t('harness.aws.setup.needsCli')}
          </span>
        )}
      </div>

      {selected && status && (
        <div className="text-sm" data-testid="aws-credentials">
          {status.valid ? (
            <div className="flex flex-col gap-1">
              <p>
                <Badge variant="secondary">{t('harness.aws.valid')}</Badge>{' '}
                {maskedIdentity(status, masked =>
                  t('harness.aws.account', { account: masked })
                ) || t(`harness.aws.kind.${selected.kind}`)}
              </p>
              {status.arn && (
                <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                  {showArn ? (
                    <code dir="ltr" className="break-all font-mono">
                      {status.arn}
                    </code>
                  ) : null}
                  <Button
                    size="sm"
                    variant="link"
                    className="h-auto p-0 text-xs"
                    onClick={() => setShowArn(value => !value)}
                  >
                    {showArn
                      ? t('harness.aws.hideArn')
                      : t('harness.aws.showArn')}
                  </Button>
                </p>
              )}
            </div>
          ) : (
            <p className="text-destructive">
              {status.expired
                ? t('harness.aws.expired')
                : (status.error ?? t('harness.aws.invalid'))}
            </p>
          )}
        </div>
      )}
      {(profiles.error || credentials.error) && (
        <p className="text-sm text-destructive">
          {errorMessage(profiles.error ?? credentials.error)}
        </p>
      )}

      <AwsKeysDialog
        open={keysFor !== null}
        profile={keysFor || null}
        onOpenChange={open => {
          if (!open) setKeysFor(null)
        }}
        onSaved={name => setChosen(name)}
      />
      <AwsSsoSetupDialog
        open={addingSso}
        onOpenChange={setAddingSso}
        onSaved={name => {
          setChosen(name)
          logIn(name)
        }}
      />
      <SsoLoginDialog
        profile={loginFor}
        loginId={loginId}
        startError={startLogin.error}
        onOpenChange={open => {
          if (!open) setLoginFor(null)
        }}
      />
    </div>
  )
}
