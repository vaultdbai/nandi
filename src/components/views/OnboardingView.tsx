import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Check } from 'lucide-react'
import { StarterPackPicker } from '@/components/marketplace/StarterPackPicker'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import {
  Step1Name,
  type IdentityDraft,
} from '@/components/onboarding/Step1_Name'
import { Step2Model } from '@/components/onboarding/Step2_Model'
import { Step3Network } from '@/components/onboarding/Step3_Network'
import {
  DEFAULT_RELAY_DRAFT,
  needsOnboarding,
  ONBOARDING_STEPS,
  publishesGateway,
  relayDraftFrom,
  relayUrlFor,
  resumeStep,
  type OnboardingStepKey,
  type RelayDraft,
} from '@/components/onboarding/identity-model'
import { gatewayQueryKeys } from '@/hooks/useGateway'
import { identityQueryKeys, useIdentity } from '@/hooks/useIdentity'
import { relayQueryKeys } from '@/hooks/useRelay'
import { client } from '@/lib/client'
import type { Result } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { logger } from '@/lib/logger'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { plainError, toastError } from '@/lib/toast-error'

export interface OnboardingViewProps {
  /**
   * Onboarding finished (or the install was onboarded already). `AuthGate`
   * passes this to open the app; without it the view navigates to the
   * dashboard.
   */
  onComplete?: () => void
}

/**
 * First launch: name (1), model (2), network (3, optional), starter packs
 * (4), then the Dashboard. Each step saves its own setting as it's left,
 * and the step reached is remembered on this Nandi home, so quitting midway
 * resumes there. The name is saved on step 1 because installing packs
 * writes agents into the repo. A relay that's already configured
 * (`NANDI_RELAY_URL` or `config.json`) is preselected and kept.
 */
export function OnboardingView({ onComplete }: OnboardingViewProps = {}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const identity = useIdentity()
  // `null` until the user moves: where the saved state says to start
  const [stepEdit, setStep] = useState<OnboardingStepKey | null>(null)
  const [name, setName] = useState<IdentityDraft>({
    name: '',
    slug: '',
    slugEdited: false,
  })
  const relayStatus = useQuery({
    queryKey: relayQueryKeys.status,
    queryFn: async () => unwrapResult(await client.getRelayStatus()),
    retry: false,
  })
  // `null` until the user picks: the configured relay, else "Not now"
  const [relayEdit, setRelay] = useState<RelayDraft | null>(null)
  const configuredRelayUrl = relayStatus.data?.relayUrl ?? null
  const builtInRelayUrl = relayStatus.data?.builtInUrl ?? null
  const relay =
    relayEdit ??
    relayDraftFrom(configuredRelayUrl, builtInRelayUrl) ??
    DEFAULT_RELAY_DRAFT
  const [savingName, setSavingName] = useState(false)
  const [finishing, setFinishing] = useState(false)
  const published = useRef(false)
  const left = useRef(false)

  const resumedAt = identity.data ? resumeStep(identity.data) : 'name'
  const step = stepEdit ?? resumedAt

  const leave = (packIds: string[]) => {
    if (left.current) return
    left.current = true
    useNavigationStore
      .getState()
      .navigate(
        packIds.length > 0
          ? { kind: 'home', welcome: { packIds } }
          : { kind: 'home' }
      )
    onComplete?.()
  }

  const alreadyOnboarded =
    identity.data !== undefined && !needsOnboarding(identity.data) && !finishing
  useEffect(() => {
    if (alreadyOnboarded) leave([])
  })

  /** Remembers the step reached; a failure only costs the resume */
  const advance = (next: OnboardingStepKey) => {
    setStep(next)
    void client.setOnboardingStep(next).then(result => {
      if (result.status === 'error') {
        logger.warn('Onboarding: saving the step failed', {
          error: result.error,
        })
      }
    })
  }

  const saveName = async () => {
    setSavingName(true)
    // The step goes first: the name makes the identity reload, and a
    // `done` left by an earlier setup would close this one if it still
    // stood then
    const writes: (() => Promise<Result<null, string>>)[] = [
      () => client.setOnboardingStep('model'),
      () => client.setDisplayName(name.name.trim()),
      () => client.setUserSlug(name.slug),
    ]
    for (const write of writes) {
      const result = await write()
      if (result.status === 'error') {
        setSavingName(false)
        toastError(
          result.error,
          t('onboarding.saveFailed', { error: plainError(result.error) })
        )
        return
      }
    }
    setSavingName(false)
    void queryClient.invalidateQueries({ queryKey: identityQueryKeys.all })
    setStep('model')
  }

  /** Registers the gateway card at the relay so others can find this Nandi */
  const publishGateway = async () => {
    if (published.current) return
    published.current = true
    const reachable = await client.makeGatewayReachable()
    const error = reachable.status === 'error' ? reachable.error : null
    // Setup goes on: the gateway can be published later in Network
    if (error !== null) {
      logger.warn('Onboarding: publishing the gateway failed', { error })
      toast.warning(t('onboarding.gatewayFailed', { error: plainError(error) }))
    }
    void queryClient.invalidateQueries({ queryKey: gatewayQueryKeys.all })
  }

  const finish = async (packIds: string[]) => {
    setFinishing(true)
    const saved = await client.setOnboardingStep('done')
    if (saved.status === 'error') {
      logger.warn('Onboarding: saving the last step failed', {
        error: saved.error,
      })
    }
    void queryClient.invalidateQueries({ queryKey: identityQueryKeys.all })
    leave(packIds)
  }

  if (identity.isPending || alreadyOnboarded) {
    return alreadyOnboarded ? null : (
      <div className="flex h-full w-full items-center justify-center">
        <Spinner className="size-6 text-muted-foreground" />
      </div>
    )
  }

  // Coming back to an unfinished setup: the rest can be skipped
  const resumed = stepEdit === null && resumedAt !== 'name'

  return (
    <div className="flex h-full w-full justify-center overflow-y-auto bg-background p-6">
      <div
        className={cn(
          'flex w-full flex-col gap-8 py-8',
          step === 'packs' ? 'max-w-3xl' : 'max-w-xl'
        )}
      >
        <header className="space-y-1">
          <h1 className="text-2xl font-semibold">{t('onboarding.title')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('onboarding.subtitle')}
          </p>
        </header>

        <StepIndicator current={step} />

        <section className="rounded-lg border p-6">
          {step === 'name' && (
            <Step1Name
              value={name}
              onChange={setName}
              saving={savingName}
              onNext={() => void saveName()}
            />
          )}
          {step === 'model' && (
            <Step2Model
              onNext={() => advance('network')}
              onBack={() => setStep('name')}
            />
          )}
          {step === 'network' && (
            <Step3Network
              value={relay}
              onChange={setRelay}
              onNext={() => {
                if (publishesGateway(relay)) void publishGateway()
                advance('packs')
              }}
              onBack={() => setStep('model')}
              configuredUrl={configuredRelayUrl}
              configuredSource={relayStatus.data?.relayUrlSource ?? null}
              builtInUrl={builtInRelayUrl}
              checking={relayStatus.isPending}
            />
          )}
          {step === 'packs' && (
            <div className="flex flex-col gap-6">
              <div className="space-y-1">
                <h2 className="text-lg font-semibold">
                  {t('onboarding.packs.title')}
                </h2>
                <p className="text-sm text-muted-foreground">
                  {t('onboarding.packs.description')}
                </p>
              </div>
              {finishing ? (
                <div className="flex justify-center py-6">
                  <Spinner className="size-6 text-muted-foreground" />
                </div>
              ) : (
                <StarterPackPicker
                  compact
                  onDone={packIds => void finish(packIds)}
                  onSkip={() => void finish([])}
                  onBack={() => setStep('network')}
                />
              )}
            </div>
          )}
        </section>

        {step === 'packs' && relayUrlFor(relay, builtInRelayUrl) === null && (
          <p className="text-xs text-muted-foreground">
            {t('onboarding.relay.skippedNote')}
          </p>
        )}
        {resumed && (
          <Button
            variant="link"
            className="self-center"
            disabled={finishing}
            onClick={() => void finish([])}
          >
            {t('onboarding.skipRest')}
          </Button>
        )}
      </div>
    </div>
  )
}

function StepIndicator({ current }: { current: OnboardingStepKey }) {
  const { t } = useTranslation()
  const currentIndex = ONBOARDING_STEPS.indexOf(current)
  return (
    <ol
      aria-label={t('onboarding.steps.label')}
      className="flex items-center gap-2"
    >
      {ONBOARDING_STEPS.map((key, index) => {
        const done = index < currentIndex
        const active = index === currentIndex
        return (
          <li
            key={key}
            aria-current={active ? 'step' : undefined}
            className="flex min-w-0 flex-1 items-center gap-2"
          >
            <span
              className={cn(
                'flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-medium',
                active && 'border-primary bg-primary text-primary-foreground',
                done && 'border-primary text-primary'
              )}
            >
              {done ? <Check className="size-3.5" /> : index + 1}
            </span>
            <span
              className={cn(
                'truncate text-sm',
                active ? 'font-medium' : 'text-muted-foreground'
              )}
            >
              {t(`onboarding.steps.${key}`)}
            </span>
            {index < ONBOARDING_STEPS.length - 1 && (
              <span className="h-px min-w-4 flex-1 bg-border" aria-hidden />
            )}
          </li>
        )
      })}
    </ol>
  )
}
