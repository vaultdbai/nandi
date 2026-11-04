import { useTranslation } from 'react-i18next'
import { CircleCheck, KeyRound, LogOut } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { ErrorMessage } from '@/components/ui/error-message'
import {
  useGithubSignIn,
  useGithubSignInFlow,
  useSignOutGithub,
} from '@/hooks/useGithubSignIn'

/**
 * Sign in with GitHub in the browser (a code to enter on
 * github.com/login/device), or who is signed in, with Sign out. The same
 * sign-in serves every github.com repository Nandi uses, so this panel
 * appears wherever one is chosen.
 */
export function GithubSignInPanel({
  showSignOut = true,
}: {
  /** Settings › Account manages it; other places only sign in */
  showSignOut?: boolean
}) {
  const { t } = useTranslation()
  const { data: status } = useGithubSignIn()
  const { state, start, cancel } = useGithubSignInFlow()
  const signOut = useSignOutGithub()

  if (state.phase === 'waiting') {
    return (
      <div
        className="space-y-1 rounded-md border p-3 text-sm"
        data-testid="github-sign-in-code"
      >
        <p>{t('github.signIn.enterCode')}</p>
        <p dir="ltr" className="font-mono text-lg tracking-widest">
          {state.flow.userCode}
        </p>
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Spinner />
          {t('github.signIn.waiting')}
        </p>
        <Button size="sm" variant="ghost" onClick={cancel}>
          {t('github.signIn.cancel')}
        </Button>
      </div>
    )
  }

  if (status?.signedIn) {
    return (
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <CircleCheck className="size-4 text-muted-foreground" />
        <span>
          {status.login
            ? t('github.signIn.signedInAs', { login: status.login })
            : t('github.signIn.signedIn')}
        </span>
        {showSignOut && (
          <Button
            size="sm"
            variant="ghost"
            disabled={signOut.isPending}
            onClick={() => signOut.mutate()}
          >
            <LogOut />
            {t('github.signIn.signOut')}
          </Button>
        )}
      </div>
    )
  }

  return (
    <div className="space-y-2">
      <Button size="sm" variant="outline" onClick={() => void start()}>
        <KeyRound />
        {t('github.signIn.button')}
      </Button>
      {state.phase === 'error' && <ErrorMessage error={state.message} />}
    </div>
  )
}
