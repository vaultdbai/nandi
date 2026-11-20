import { useTranslation } from 'react-i18next'
import { ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { ErrorMessage } from '@/components/ui/error-message'
import {
  useCheckGithubSecurity,
  useGithubSecurityStatus,
} from '@/hooks/useGithubSignIn'
import { securityLine } from './github-security-model'

/**
 * Settings › Account, under the GitHub sign-in: Check GitHub security, and
 * when it last ran. Each finding is a to-do in the Inbox. Read-only: Nandi
 * reads settings, never secrets or repository contents.
 */
export function GithubSecurityCheck() {
  const { t, i18n } = useTranslation()
  const { data: status } = useGithubSecurityStatus()
  const check = useCheckGithubSecurity()
  const line = securityLine(status, i18n.language)

  return (
    <div className="space-y-2" data-testid="github-security-check">
      <p className="text-sm text-muted-foreground">
        {t('github.security.description')}
      </p>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Button
          size="sm"
          variant="outline"
          disabled={check.isPending}
          onClick={() => check.mutate()}
        >
          {check.isPending ? <Spinner /> : <ShieldCheck />}
          {t('github.security.button')}
        </Button>
        <span className="text-muted-foreground">
          {t(line.key, line.values)}
        </span>
      </div>
      {check.error && (
        <ErrorMessage error={check.error} message={check.error.message} />
      )}
    </div>
  )
}
