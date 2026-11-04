import { useEffect, useState } from 'react'
import { needsOnboarding } from '@/components/onboarding/identity-model'
import { Spinner } from '@/components/ui/spinner'
import { OnboardingView } from '@/components/views/OnboardingView'
import { client } from '@/lib/client'
import { logger } from '@/lib/logger'

type GatePhase = 'loading' | 'onboarding' | 'ready'

/**
 * Routes on local identity (`~/.nandi/repo/NANDI.md`): not onboarded, or a
 * first-run setup that stopped midway → `OnboardingView`; otherwise the
 * app (`children`).
 *
 * Fails open: if the identity status can't be read, the app opens anyway
 * (and the error is logged), so a core error never traps the user on
 * onboarding.
 */
export function AuthGate({ children }: { children: React.ReactNode }) {
  const [phase, setPhase] = useState<GatePhase>('loading')

  useEffect(() => {
    let cancelled = false
    const failOpen = (error: unknown) => {
      logger.warn('Failed to load identity status; opening the app', {
        error,
      })
      setPhase('ready')
    }
    client.getIdentityStatus().then(
      result => {
        if (cancelled) return
        if (result.status === 'error') {
          failOpen(result.error)
          return
        }
        setPhase(needsOnboarding(result.data) ? 'onboarding' : 'ready')
      },
      (error: unknown) => {
        if (!cancelled) failOpen(error)
      }
    )
    return () => {
      cancelled = true
    }
  }, [])

  switch (phase) {
    case 'loading':
      return (
        <div className="flex h-full w-full items-center justify-center">
          <Spinner className="size-6 text-muted-foreground" />
        </div>
      )
    case 'onboarding':
      return (
        <div className="h-full w-full" data-testid="onboarding-gate">
          <OnboardingView onComplete={() => setPhase('ready')} />
        </div>
      )
    case 'ready':
      return children
  }
}
