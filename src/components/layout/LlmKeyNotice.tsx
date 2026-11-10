import { useTranslation } from 'react-i18next'
import { KeyRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { openSettings } from '@/lib/settings-nav'
import { useUIStore } from '@/store/ui-store'
import { cn } from '@/lib/utils'

/**
 * Shown above the message box (and on the Dashboard) while no model can
 * be called: no key is set for the default model's provider or any other
 * (Ollama needs none). Its button opens Settings › Models & runtimes;
 * "How?" opens the built-in help guide, which works with no model.
 */
export function LlmKeyNotice({ className }: { className?: string }) {
  const { t } = useTranslation()
  const { data: status } = useDefaultModel()

  // Unknown (loading/error) or callable: stay out of the way
  if (!status || status.effectiveModel !== null) return null

  return (
    <div
      role="note"
      className={cn(
        'mx-4 mt-3 flex shrink-0 items-center gap-3 rounded-lg border border-warning/30 bg-warning/5 px-3 py-2 text-sm',
        className
      )}
    >
      <KeyRound aria-hidden className="size-4 shrink-0 text-warning" />
      <p className="min-w-0 flex-1 text-foreground/90">
        {t(
          status.trial?.usedUp
            ? 'llmKeyNotice.trialUsedUp'
            : status.trial?.paused
              ? 'llmKeyNotice.trialPaused'
              : status.trial?.unreachable
                ? 'llmKeyNotice.trialUnreachable'
                : 'llmKeyNotice.message'
        )}
      </p>
      <Button
        size="sm"
        variant="link"
        className="h-7 shrink-0 px-1"
        onClick={() => useUIStore.getState().openHelp('connect-model')}
      >
        {t('help.how')}
      </Button>
      <Button
        size="sm"
        variant="outline"
        className="h-7 shrink-0"
        onClick={() => openSettings('models')}
      >
        {t('llmKeyNotice.action')}
      </Button>
    </div>
  )
}
