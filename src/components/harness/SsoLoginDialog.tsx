import { useTranslation } from 'react-i18next'
import { CircleCheck, CircleX, ExternalLink } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Spinner } from '@/components/ui/spinner'
import { rawError } from '@/lib/friendly-error'
import { useSsoLogin } from '@/hooks/useAwsAuth'
import { client } from '@/lib/client'

/**
 * An `aws sso login` in progress for one profile (started by the caller):
 * the AWS CLI opens the browser; this shows the code and link it printed
 * and waits for the approval.
 */
export function SsoLoginDialog({
  profile,
  loginId,
  startError,
  onOpenChange,
}: {
  profile: string | null
  /** `null` while the login is starting */
  loginId: string | null
  startError?: unknown
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const login = useSsoLogin(loginId)

  const result = login.data
  const status = result?.status ?? 'pending'
  const error = startError ?? login.error

  return (
    <Dialog open={profile !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('harness.aws.sso.title', { profile })}</DialogTitle>
          <DialogDescription>
            {t('harness.aws.sso.description')}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3 text-sm" data-testid="sso-login">
          {error ? (
            <p className="text-destructive">{rawError(error)}</p>
          ) : status === 'pending' ? (
            <p className="flex items-center gap-2">
              <Spinner />
              {t('harness.aws.sso.waiting')}
            </p>
          ) : status === 'succeeded' ? (
            <p className="flex items-center gap-2">
              <CircleCheck className="size-4" />
              {t('harness.aws.sso.succeeded')}
            </p>
          ) : (
            <p className="flex items-center gap-2 text-destructive">
              <CircleX className="size-4" />
              {result?.error ?? t('harness.aws.sso.failed')}
            </p>
          )}
          {result?.userCode && (
            <p>
              {t('harness.aws.sso.code')}{' '}
              <code className="rounded bg-muted px-1.5 py-0.5 font-mono">
                {result.userCode}
              </code>
            </p>
          )}
          {result?.verificationUri && status === 'pending' && (
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              onClick={() =>
                void client.openExternal(result.verificationUri ?? '')
              }
            >
              <ExternalLink />
              {t('harness.aws.sso.open')}
            </Button>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {status === 'pending'
              ? t('harness.aws.sso.cancel')
              : t('harness.aws.sso.close')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
