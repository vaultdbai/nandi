import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronDown } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { client } from '@/lib/client'
import type { KeyedProvider } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { planView, PLANS, type PlanAction, type PlanId } from './plans-model'

export interface PlansChooserProps {
  /** The build offers the free tokens */
  trialOffered: boolean
  /** Opens the key form of a provider already on the step */
  onPickProvider: (provider: KeyedProvider) => void
  /** Opens the Ollama setup already on the step */
  onPickOllama: () => void
}

/**
 * "What do you already use?": an optional, collapsed help. The person picks
 * a plan; it says plainly what that plan allows in a few steps. No plan
 * detection: it only reads what they pick.
 */
export function PlansChooser({
  trialOffered,
  onPickProvider,
  onPickOllama,
}: PlansChooserProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [plan, setPlan] = useState<PlanId | null>(null)
  const view = plan ? planView(plan, trialOffered) : null

  const run = (action: PlanAction) => {
    if (action.kind === 'link') void client.openExternal(action.url)
    else if (action.kind === 'provider') onPickProvider(action.provider)
    else onPickOllama()
  }

  return (
    <div className="flex flex-col gap-2" data-testid="plans-chooser">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 self-start text-sm font-medium text-muted-foreground hover:text-foreground"
      >
        <ChevronDown
          aria-hidden
          className={cn('size-4 transition-transform', !open && '-rotate-90')}
        />
        {t('onboarding.plans.title')}
      </button>
      {open && (
        <div className="flex flex-col gap-3 rounded-md border p-3">
          <p className="text-xs text-muted-foreground">
            {t('onboarding.plans.description')}
          </p>
          <div
            role="radiogroup"
            aria-label={t('onboarding.plans.title')}
            className="flex flex-wrap gap-2"
          >
            {PLANS.map(({ id }) => (
              <Button
                key={id}
                type="button"
                role="radio"
                aria-checked={plan === id}
                size="sm"
                variant={plan === id ? 'default' : 'outline'}
                onClick={() => setPlan(id)}
              >
                {t(`onboarding.plans.${id}.name`)}
              </Button>
            ))}
          </div>
          {view && (
            <div className="flex flex-col gap-2" data-testid="plan-steps">
              <ol className="list-decimal space-y-1 ps-5 text-sm">
                {view.stepKeys.map(key => (
                  <li key={key}>{t(key)}</li>
                ))}
              </ol>
              {view.actions.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {view.actions.map(action => (
                    <Button
                      key={action.labelKey}
                      type="button"
                      size="sm"
                      variant="outline"
                      onClick={() => run(action)}
                    >
                      {t(action.labelKey)}
                    </Button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
