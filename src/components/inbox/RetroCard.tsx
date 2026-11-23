import { useTranslation } from 'react-i18next'
import { Lightbulb } from 'lucide-react'
import { toast } from 'sonner'
import { DiffLines } from '@/components/agent-console/DiffLines'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { useViewOpener } from '@/hooks/useViewOpener'
import { useRetroAction } from '@/hooks/useRetro'
import type { RetroAction, RetroItem } from '@/lib/bindings'
import {
  ratePercent,
  retroCardState,
  retroDiffLines,
  retroTargetLabel,
} from './retro-model'

/**
 * A weekly retro item (`services::retro`): what Nandi proposes to change,
 * why, the diff and the runs it learned from, with Apply and No thanks.
 * Once applied: Undo. When the check found it didn't help: the rates
 * before and after, with Undo and Keep it.
 */
export function RetroCard({ item }: { item: RetroItem }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const act = useRetroAction()
  const state = retroCardState(item)
  const target = retroTargetLabel(item.target)
  const run = (action: RetroAction, done: string) =>
    act.mutate(
      { id: item.id, action },
      {
        onSuccess: () => toast.success(t(done)),
        onError: error => toast.error(error.message),
      }
    )

  return (
    <Card
      data-testid="retro-card"
      className="flex-col items-stretch gap-2 px-4 py-3 shadow-none"
    >
      <div className="flex items-start gap-3">
        <Lightbulb
          aria-hidden
          className="mt-0.5 size-4 shrink-0 text-primary"
        />
        <div className="min-w-0 flex-1 space-y-1 text-sm">
          <p className="font-medium break-words text-foreground">
            {t(`retro.card.${state}`, { agent: item.agentName })}
          </p>
          <p className="text-muted-foreground">
            {t(target.key, target.values)}
          </p>
          <p className="break-words text-foreground">
            {t('retro.card.why', { reason: item.reason })}
          </p>
          {state === 'didntHelp' && item.rates && (
            <p className="text-muted-foreground" data-testid="retro-rates">
              {t('retro.card.rates', {
                before: ratePercent(item.rates.before),
                after: ratePercent(item.rates.after),
                decisions: item.rates.after.decisions,
              })}
            </p>
          )}
        </div>
      </div>

      <Collapsible>
        <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-2 hover:underline">
          {t('retro.card.showChange')}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <pre
            dir="ltr"
            data-testid="retro-diff"
            className="mt-2 max-h-72 overflow-auto rounded-md border bg-muted/60 px-3 py-2 text-start font-mono text-xs whitespace-pre-wrap break-words text-foreground"
          >
            <DiffLines lines={retroDiffLines(item.diff)} />
          </pre>
        </CollapsibleContent>
      </Collapsible>

      {item.runs.length > 0 && (
        <p className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
          {t('retro.card.runs')}
          {item.runs.map((runId, index) => (
            <Button
              key={runId}
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={() => open({ kind: 'run-detail', runId })}
            >
              {t('retro.card.run', { number: index + 1 })}
            </Button>
          ))}
        </p>
      )}

      <div className="flex justify-end gap-2">
        {state === 'proposal' && (
          <>
            <Button
              size="sm"
              variant="outline"
              disabled={act.isPending}
              onClick={() => run('refuse', 'retro.toast.refused')}
            >
              {t('retro.card.noThanks')}
            </Button>
            <Button
              size="sm"
              disabled={act.isPending}
              onClick={() => run('apply', 'retro.toast.applied')}
            >
              {t('retro.card.apply')}
            </Button>
          </>
        )}
        {state === 'didntHelp' && (
          <Button
            size="sm"
            variant="outline"
            disabled={act.isPending}
            onClick={() => run('keep', 'retro.toast.kept')}
          >
            {t('retro.card.keep')}
          </Button>
        )}
        {state !== 'proposal' && (
          <Button
            size="sm"
            variant={state === 'didntHelp' ? 'default' : 'outline'}
            disabled={act.isPending}
            onClick={() => run('undo', 'retro.toast.undone')}
          >
            {t('retro.card.undo')}
          </Button>
        )}
      </div>
    </Card>
  )
}
