import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import { CircleCheck, CircleSlash } from 'lucide-react'
import { toast } from 'sonner'
import { AwsAuthPanel } from '@/components/harness/AwsAuthPanel'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { egressStatusQueryKey, useEgressStatus } from '@/hooks/useEgressStatus'
import { client } from '@/lib/client'
import type { AwsCredentialStatus, Result } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import {
  AdvancedBlock,
  SettingsGroup,
  SettingsSectionFrame,
} from '../SettingsFrame'

/**
 * Settings › Cloud credentials: the one place for AWS. The profile / SSO
 * sign-in serves Bedrock models, and agents' AWS calls when there's no
 * access key; the access key serves agents' AWS calls through Nandi. An
 * agent may name its own profile instead (its Settings tab).
 */
export function CloudCredentialsSection() {
  const { t } = useTranslation()
  const { data: status, error } = useEgressStatus()

  return (
    <SettingsSectionFrame
      title={t('settings.section.cloud')}
      description={t('settings.cloud.description')}
    >
      <SettingsGroup
        title={t('settings.cloud.aws')}
        description={t('settings.cloud.usedFor')}
      >
        <div className="flex flex-col gap-3 rounded-lg border p-4">
          <h4 className="text-sm font-medium">{t('settings.cloud.signIn')}</h4>
          <p className="text-sm text-muted-foreground">
            {t('settings.cloud.signInHint')}
          </p>
          <AwsAuthPanel />
        </div>
        <div
          className="flex flex-col gap-3 rounded-lg border p-4"
          data-settings-focus="aws-access-key"
        >
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {t('settings.cloud.loadFailed')}
            </p>
          )}
          {status ? (
            <AccessKeyForm aws={status.aws} />
          ) : (
            !error && <Skeleton className="h-40" />
          )}
        </div>
      </SettingsGroup>
    </SettingsSectionFrame>
  )
}

/** A write-only access key and region for agents' AWS calls */
function AccessKeyForm({ aws }: { aws: AwsCredentialStatus }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [accessKey, setAccessKey] = useState('')
  const [secret, setSecret] = useState('')
  const [region, setRegion] = useState('')
  const [saving, setSaving] = useState(false)
  const [confirmRemove, setConfirmRemove] = useState(false)

  const run = async (
    action: () => Promise<Result<null, string>>,
    success: string
  ) => {
    setSaving(true)
    const result = await action()
    setSaving(false)
    if (result.status === 'error') {
      toast.error(t('settings.cloud.saveFailed', { error: result.error }))
      return false
    }
    toast.success(t(success))
    await queryClient.invalidateQueries({ queryKey: egressStatusQueryKey })
    return true
  }

  const saveCredentials = async () => {
    const saved = await run(
      () => client.setAwsCredentials(accessKey.trim(), secret),
      'egress.aws.saved'
    )
    if (saved) {
      setAccessKey('')
      setSecret('')
    }
  }

  const saveRegion = async () => {
    if (
      await run(
        () => client.setAwsRegion(region.trim()),
        'egress.aws.regionSaved'
      )
    )
      setRegion('')
  }

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="text-sm font-medium">{t('settings.cloud.accessKey')}</h4>
        <Badge
          variant="outline"
          className={cn(
            'gap-1',
            aws.configured ? 'text-success' : 'text-muted-foreground'
          )}
        >
          {aws.configured ? (
            <CircleCheck className="size-3" />
          ) : (
            <CircleSlash className="size-3" />
          )}
          {t(
            aws.configured
              ? 'egress.aws.configured'
              : 'egress.aws.notConfigured'
          )}
        </Badge>
      </div>
      <p className="text-sm text-muted-foreground">
        {t('settings.cloud.accessKeyHint')}
      </p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted-foreground">{t('egress.aws.accessKey')}</dt>
        <dd className="font-mono" dir="ltr">
          {aws.accessKeyHint ?? '—'}
        </dd>
        <dt className="text-muted-foreground">{t('egress.aws.region')}</dt>
        <dd className="font-mono" dir="ltr">
          {aws.region}
        </dd>
      </dl>

      <form
        className="flex flex-col gap-2"
        onSubmit={event => {
          event.preventDefault()
          if (accessKey.trim() && secret) void saveCredentials()
        }}
      >
        <Label htmlFor="aws-access-key">{t('egress.aws.accessKey')}</Label>
        <Input
          id="aws-access-key"
          dir="ltr"
          autoComplete="off"
          spellCheck={false}
          value={accessKey}
          onChange={event => setAccessKey(event.target.value)}
          placeholder="AKIA…"
        />
        <Label htmlFor="aws-secret">{t('egress.aws.secret')}</Label>
        <Input
          id="aws-secret"
          type="password"
          dir="ltr"
          autoComplete="off"
          spellCheck={false}
          value={secret}
          onChange={event => setSecret(event.target.value)}
          placeholder={t('egress.aws.secretPlaceholder')}
        />
        <div className="flex gap-2">
          <Button
            type="submit"
            size="sm"
            disabled={saving || !accessKey.trim() || !secret}
          >
            {t('egress.aws.save')}
          </Button>
          {aws.configured && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={saving}
              onClick={() => setConfirmRemove(true)}
            >
              {t('egress.aws.remove')}
            </Button>
          )}
        </div>
      </form>

      <form
        className="flex items-end gap-2"
        onSubmit={event => {
          event.preventDefault()
          if (region.trim()) void saveRegion()
        }}
      >
        <div className="flex flex-1 flex-col gap-2">
          <Label htmlFor="aws-region">{t('egress.aws.region')}</Label>
          <Input
            id="aws-region"
            dir="ltr"
            autoComplete="off"
            spellCheck={false}
            value={region}
            onChange={event => setRegion(event.target.value)}
            placeholder={aws.region}
          />
        </div>
        <Button
          type="submit"
          size="sm"
          variant="outline"
          disabled={saving || !region.trim()}
        >
          {t('egress.aws.setRegion')}
        </Button>
      </form>

      <AdvancedBlock>
        <p className="text-sm text-muted-foreground">
          {t('settings.cloud.sessions', { count: aws.activeSessions })}
        </p>
      </AdvancedBlock>

      <AlertDialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('settings.cloud.removeConfirm.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.cloud.removeConfirm.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('remote.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                void run(
                  () => client.setAwsCredentials('', ''),
                  'egress.aws.removed'
                )
              }
            >
              {t('settings.cloud.removeConfirm.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
