import { useState } from 'react'
import { useTranslation } from 'react-i18next'
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
import { Spinner } from '@/components/ui/spinner'
import { rawError } from '@/lib/friendly-error'
import { useSaveKeyProfile } from '@/hooks/useAwsAuth'
import { profileNameError, regionError } from './aws-setup-model'

/**
 * Access keys for an AWS profile: a new profile, or new keys for `profile`.
 * The AWS CLI writes them to the user's AWS files (`aws configure`); the
 * secret goes to it on stdin and is never shown again.
 */
export function AwsKeysDialog({
  open,
  profile,
  onOpenChange,
  onSaved,
}: {
  open: boolean
  /** Replacing this profile's keys; `null`: a new profile */
  profile: string | null
  onOpenChange: (open: boolean) => void
  onSaved: (profile: string) => void
}) {
  const { t } = useTranslation()
  const save = useSaveKeyProfile()
  const [name, setName] = useState('')
  const [keyId, setKeyId] = useState('')
  const [secret, setSecret] = useState('')
  const [region, setRegion] = useState('')

  const target = profile ?? name.trim()
  const nameProblem = profile ? null : profileNameError(name)
  const regionProblem = regionError(region)
  const ready =
    !nameProblem &&
    !regionProblem &&
    keyId.trim() !== '' &&
    secret.trim() !== ''

  const close = (next: boolean) => {
    if (!next) {
      setName('')
      setKeyId('')
      setSecret('')
      setRegion('')
      save.reset()
    }
    onOpenChange(next)
  }

  const submit = () =>
    save.mutate(
      {
        profile: target,
        accessKeyId: keyId.trim(),
        secretAccessKey: secret,
        region: region.trim() || null,
      },
      {
        onSuccess: saved => {
          close(false)
          onSaved(saved.name)
        },
      }
    )

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {profile
              ? t('harness.aws.keys.replaceTitle', { profile })
              : t('harness.aws.keys.addTitle')}
          </DialogTitle>
          <DialogDescription>
            {t('harness.aws.keys.description')}
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-3"
          onSubmit={event => {
            event.preventDefault()
            if (ready) submit()
          }}
        >
          {!profile && (
            <div className="space-y-1.5">
              <Label htmlFor="aws-keys-name">
                {t('harness.aws.setup.profileName')}
              </Label>
              <Input
                id="aws-keys-name"
                dir="ltr"
                value={name}
                placeholder="work"
                onChange={event => setName(event.target.value)}
              />
              {name !== '' && nameProblem && (
                <p className="text-xs text-destructive">{t(nameProblem)}</p>
              )}
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="aws-keys-id">{t('harness.aws.keys.keyId')}</Label>
            <Input
              id="aws-keys-id"
              dir="ltr"
              autoComplete="off"
              value={keyId}
              placeholder="AKIA…"
              onChange={event => setKeyId(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="aws-keys-secret">
              {t('harness.aws.keys.secret')}
            </Label>
            <Input
              id="aws-keys-secret"
              type="password"
              dir="ltr"
              autoComplete="off"
              value={secret}
              onChange={event => setSecret(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="aws-keys-region">
              {t('harness.aws.setup.region')}
            </Label>
            <Input
              id="aws-keys-region"
              dir="ltr"
              value={region}
              placeholder={
                profile ? t('harness.aws.keys.keepRegion') : 'us-east-1'
              }
              onChange={event => setRegion(event.target.value)}
            />
            {regionProblem && (
              <p className="text-xs text-destructive">{t(regionProblem)}</p>
            )}
          </div>
          {save.error && (
            <p role="alert" className="text-sm text-destructive">
              {rawError(save.error)}
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
            <Button type="submit" disabled={!ready || save.isPending}>
              {save.isPending && <Spinner />}
              {t('harness.aws.keys.save')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
