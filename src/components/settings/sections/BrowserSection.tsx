import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Globe, LogOut } from 'lucide-react'
import { toast } from 'sonner'
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
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useAgents } from '@/hooks/useAgents'
import {
  useNandiBrowsers,
  useOpenNandiBrowser,
  useSignOutNandiBrowser,
} from '@/hooks/usePilots'
import type { NandiBrowser } from '@/lib/bindings'
import { BROWSER_LABELS, nandiBrowserState } from '@/lib/browser-profile'
import { SettingsSectionFrame } from '../SettingsFrame'

/** Commands reject with the core's error string, not an `Error` */
function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/**
 * Settings › Browser: My Nandi browser, one shared profile per installed
 * browser type. Open my Nandi browser opens the person's own window on it
 * (no agent attached) to sign in to sites; Sign out everywhere wipes it.
 * Agents use it only when the person chooses it on the agent's page.
 */
export function BrowserSection() {
  const { t } = useTranslation()
  const browsers = useNandiBrowsers()
  return (
    <SettingsSectionFrame
      title={t('settings.section.browser')}
      description={t('settings.browser.description')}
    >
      {browsers.isPending && <Skeleton className="h-32 w-full" />}
      {browsers.isError && (
        <Alert variant="destructive">
          <AlertDescription>
            {t('settings.browser.loadFailed')}
          </AlertDescription>
        </Alert>
      )}
      {browsers.isSuccess && browsers.data.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t('settings.browser.none')}
        </p>
      )}
      {browsers.data?.map(browser => (
        <BrowserCard key={browser.browser} browser={browser} />
      ))}
    </SettingsSectionFrame>
  )
}

function BrowserCard({ browser }: { browser: NandiBrowser }) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  const open = useOpenNandiBrowser()
  const label = BROWSER_LABELS[browser.browser]
  const nameOf = (id: string) => agents.find(a => a.id === id)?.name ?? id
  const state = nandiBrowserState(browser)
  return (
    <section
      className="flex flex-col gap-3 rounded-lg border p-4"
      aria-label={t('settings.browser.title', { browser: label })}
    >
      <div className="flex flex-wrap items-start gap-3">
        <Globe className="mt-0.5 size-5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold">
            {t('settings.browser.title', { browser: label })}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t(`settings.browser.state.${state}`, {
              name: browser.inUseBy ? nameOf(browser.inUseBy) : '',
            })}
          </p>
        </div>
        <Button
          size="sm"
          disabled={open.isPending || state === 'inUse'}
          onClick={() =>
            open.mutate(browser.browser, {
              onSuccess: () =>
                toast.success(t('settings.browser.opened', { browser: label })),
              onError: error =>
                toast.error(t('settings.browser.openFailed'), {
                  description: errorText(error),
                }),
            })
          }
        >
          {open.isPending && <Spinner />}
          {t('settings.browser.open')}
        </Button>
        <SignOutButton browser={browser} label={label} />
      </div>
      <p className="text-sm">
        {t('settings.browser.syncNote', { browser: label })}
      </p>
      <p className="text-xs text-muted-foreground">
        {t('settings.browser.ownBrowsing')}
      </p>
      <p className="text-xs text-muted-foreground">
        {browser.agents.length > 0
          ? t('settings.browser.agents', {
              names: browser.agents.map(nameOf).join(', '),
            })
          : t('settings.browser.noAgents')}
      </p>
    </section>
  )
}

function SignOutButton({
  browser,
  label,
}: {
  browser: NandiBrowser
  label: string
}) {
  const { t } = useTranslation()
  const signOut = useSignOutNandiBrowser()
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        disabled={!browser.hasProfile || signOut.isPending}
        onClick={() => setConfirming(true)}
      >
        <LogOut />
        {t('settings.browser.signOut')}
      </Button>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('settings.browser.signOutConfirmTitle', { browser: label })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.browser.signOutConfirmDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('settings.browser.signOutCancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                signOut.mutate(browser.browser, {
                  onSuccess: () =>
                    toast.success(t('settings.browser.signedOut')),
                  onError: error =>
                    toast.error(t('settings.browser.signOutFailed'), {
                      description: errorText(error),
                    }),
                })
              }
            >
              {t('settings.browser.signOut')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
