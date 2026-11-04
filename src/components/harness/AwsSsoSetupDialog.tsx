import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CircleX, ExternalLink } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { errorMessage } from '@/hooks/useDelegation'
import {
  useFinishSsoSetup,
  useSsoSetup,
  useStartSsoSetup,
} from '@/hooks/useAwsAuth'
import type { SsoAccount } from '@/lib/bindings'
import { client } from '@/lib/client'
import {
  profileNameError,
  regionError,
  startUrlError,
  suggestedProfileName,
} from './aws-setup-model'

/**
 * A new SSO (IAM Identity Center) profile, in three steps: the access
 * portal's start URL and region; approving the sign-in in the browser; then
 * the account, role and profile name. Saving writes the profile through the
 * AWS CLI, and `onSaved` then signs the CLI in (`aws sso login`).
 */
export function AwsSsoSetupDialog({
  open,
  onOpenChange,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSaved: (profile: string) => void
}) {
  const { t } = useTranslation()
  const start = useStartSsoSetup()
  const finish = useFinishSsoSetup()
  const [startUrl, setStartUrl] = useState('')
  const [ssoRegion, setSsoRegion] = useState('us-east-1')
  const [setupId, setSetupId] = useState<string | null>(null)
  const setup = useSsoSetup(setupId)
  const status = setup.data?.status

  const reset = () => {
    if (setupId) void client.cancelAwsSsoSetup(setupId)
    setSetupId(null)
    start.reset()
    finish.reset()
  }
  const close = (next: boolean) => {
    if (!next) {
      reset()
      setStartUrl('')
    }
    onOpenChange(next)
  }

  // The sign-in page opens by itself once, as `aws sso login` does
  const opened = useRef<string | null>(null)
  const verificationUri = setup.data?.verificationUri ?? null
  useEffect(() => {
    if (verificationUri && opened.current !== setupId) {
      opened.current = setupId
      void client.openExternal(verificationUri)
    }
  }, [verificationUri, setupId])

  const urlProblem = startUrl.trim() ? startUrlError(startUrl) : null
  const ssoRegionProblem =
    regionError(ssoRegion) ??
    (ssoRegion.trim() ? null : 'harness.aws.setup.errors.region')
  const begin = () =>
    start.mutate(
      { startUrl: startUrl.trim(), ssoRegion: ssoRegion.trim() },
      { onSuccess: started => setSetupId(started.setupId) }
    )

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('harness.aws.setup.ssoTitle')}</DialogTitle>
          <DialogDescription>
            {t('harness.aws.setup.ssoDescription')}
          </DialogDescription>
        </DialogHeader>

        {!setupId ? (
          <form
            className="flex flex-col gap-3"
            onSubmit={event => {
              event.preventDefault()
              if (!urlProblem && !ssoRegionProblem && startUrl.trim()) begin()
            }}
          >
            <div className="space-y-1.5">
              <Label htmlFor="aws-sso-url">
                {t('harness.aws.setup.startUrl')}
              </Label>
              <Input
                id="aws-sso-url"
                dir="ltr"
                value={startUrl}
                placeholder="https://my-company.awsapps.com/start"
                onChange={event => setStartUrl(event.target.value)}
              />
              {urlProblem && (
                <p className="text-xs text-destructive">{t(urlProblem)}</p>
              )}
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="aws-sso-region">
                {t('harness.aws.setup.ssoRegion')}
              </Label>
              <Input
                id="aws-sso-region"
                dir="ltr"
                value={ssoRegion}
                onChange={event => setSsoRegion(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                {t('harness.aws.setup.ssoRegionHint')}
              </p>
            </div>
            {start.error && (
              <p role="alert" className="text-sm text-destructive">
                {errorMessage(start.error)}
              </p>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => close(false)}
              >
                {t('harness.aws.sso.cancel')}
              </Button>
              <Button
                type="submit"
                disabled={
                  !startUrl.trim() ||
                  !!urlProblem ||
                  !!ssoRegionProblem ||
                  start.isPending
                }
              >
                {start.isPending && <Spinner />}
                {t('harness.aws.setup.signIn')}
              </Button>
            </DialogFooter>
          </form>
        ) : status === 'ready' && setup.data ? (
          <PickRole
            accounts={setup.data.accounts}
            ssoRegion={setup.data.ssoRegion}
            saving={finish.isPending}
            error={finish.error}
            onCancel={() => close(false)}
            onSave={pick =>
              finish.mutate(
                { setupId, ...pick },
                {
                  onSuccess: saved => {
                    // The setup is used up: nothing to cancel
                    setSetupId(null)
                    onOpenChange(false)
                    setStartUrl('')
                    onSaved(saved.name)
                  },
                }
              )
            }
          />
        ) : status === 'failed' || status === 'expired' || setup.error ? (
          <div className="flex flex-col gap-3 text-sm">
            <p className="flex items-center gap-2 text-destructive">
              <CircleX className="size-4" />
              {setup.data?.error ?? errorMessage(setup.error)}
            </p>
            <DialogFooter>
              <Button variant="outline" onClick={() => close(false)}>
                {t('harness.aws.sso.close')}
              </Button>
              <Button onClick={reset}>{t('harness.aws.setup.tryAgain')}</Button>
            </DialogFooter>
          </div>
        ) : (
          <div
            className="flex flex-col gap-3 text-sm"
            data-testid="sso-setup-waiting"
          >
            <p className="flex items-center gap-2">
              <Spinner />
              {t('harness.aws.setup.waiting')}
            </p>
            {setup.data?.userCode && (
              <p>
                {t('harness.aws.sso.code')}{' '}
                <code className="rounded bg-muted px-1.5 py-0.5 font-mono">
                  {setup.data.userCode}
                </code>
              </p>
            )}
            {verificationUri && (
              <Button
                variant="outline"
                size="sm"
                className="self-start"
                onClick={() => void client.openExternal(verificationUri)}
              >
                <ExternalLink />
                {t('harness.aws.sso.open')}
              </Button>
            )}
            <DialogFooter>
              <Button variant="outline" onClick={() => close(false)}>
                {t('harness.aws.sso.cancel')}
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

/** The account, role, profile name and region for the new profile */
function PickRole({
  accounts,
  ssoRegion,
  saving,
  error,
  onCancel,
  onSave,
}: {
  accounts: SsoAccount[]
  ssoRegion: string
  saving: boolean
  error: unknown
  onCancel: () => void
  onSave: (pick: {
    profile: string
    accountId: string
    roleName: string
    region: string | null
  }) => void
}) {
  const { t } = useTranslation()
  const [accountId, setAccountId] = useState(accounts[0]?.accountId ?? '')
  const account = accounts.find(a => a.accountId === accountId)
  const [roleName, setRoleName] = useState(account?.roles[0] ?? '')
  const [name, setName] = useState<string | null>(null)
  const [region, setRegion] = useState('')
  const suggested =
    account && roleName ? suggestedProfileName(account, roleName) : ''
  const profile = name ?? suggested
  const nameProblem = profileNameError(profile)
  const regionProblem = regionError(region)

  if (accounts.length === 0) {
    return (
      <div className="flex flex-col gap-3 text-sm">
        <p className="text-destructive">{t('harness.aws.setup.noAccounts')}</p>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {t('harness.aws.sso.close')}
          </Button>
        </DialogFooter>
      </div>
    )
  }

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={event => {
        event.preventDefault()
        if (!nameProblem && !regionProblem && roleName)
          onSave({
            profile: profile.trim(),
            accountId,
            roleName,
            region: region.trim() || null,
          })
      }}
    >
      <p className="text-sm text-muted-foreground">
        {t('harness.aws.setup.signedIn')}
      </p>
      <div className="space-y-1.5">
        <Label htmlFor="aws-sso-account">
          {t('harness.aws.setup.account')}
        </Label>
        <NativeSelect
          id="aws-sso-account"
          value={accountId}
          onChange={event => {
            const next = accounts.find(a => a.accountId === event.target.value)
            setAccountId(event.target.value)
            setRoleName(next?.roles[0] ?? '')
          }}
        >
          {accounts.map(a => (
            <NativeSelectOption key={a.accountId} value={a.accountId}>
              {a.accountName} ({a.accountId})
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="aws-sso-role">{t('harness.aws.setup.role')}</Label>
        <NativeSelect
          id="aws-sso-role"
          value={roleName}
          onChange={event => setRoleName(event.target.value)}
        >
          {(account?.roles ?? []).map(role => (
            <NativeSelectOption key={role} value={role}>
              {role}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="aws-sso-name">
          {t('harness.aws.setup.profileName')}
        </Label>
        <Input
          id="aws-sso-name"
          dir="ltr"
          value={profile}
          onChange={event => setName(event.target.value)}
        />
        {nameProblem && (
          <p className="text-xs text-destructive">{t(nameProblem)}</p>
        )}
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="aws-sso-profile-region">
          {t('harness.aws.setup.region')}
        </Label>
        <Input
          id="aws-sso-profile-region"
          dir="ltr"
          value={region}
          placeholder={ssoRegion}
          onChange={event => setRegion(event.target.value)}
        />
        {regionProblem && (
          <p className="text-xs text-destructive">{t(regionProblem)}</p>
        )}
      </div>
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(error)}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">
        {t('harness.aws.setup.loginNext')}
      </p>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onCancel}>
          {t('harness.aws.sso.cancel')}
        </Button>
        <Button
          type="submit"
          disabled={!!nameProblem || !!regionProblem || !roleName || saving}
        >
          {saving && <Spinner />}
          {t('harness.aws.setup.saveAndLogIn')}
        </Button>
      </DialogFooter>
    </form>
  )
}
