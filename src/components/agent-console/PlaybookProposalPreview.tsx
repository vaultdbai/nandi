import { useTranslation } from 'react-i18next'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { cn } from '@/lib/utils'
import {
  changedLines,
  lineDiff,
  type PlaybookProposal,
} from './playbook-proposal-model'

interface PlaybookProposalPreviewProps {
  proposal: PlaybookProposal
}

/**
 * The playbook an agent proposes, on its approval card: why, what it
 * covers, when, and the steps (folded). A change shows what it replaces:
 * the old one-line fields and the steps as a line diff.
 */
export function PlaybookProposalPreview({
  proposal,
}: PlaybookProposalPreviewProps) {
  const { t } = useTranslation()
  const changed = changedLines(proposal)
  const was = (key: 'description' | 'when') =>
    proposal.current && changed.includes(key)
      ? proposal.current[key]
      : undefined

  return (
    <div data-testid="playbook-proposal" className="space-y-2 text-sm">
      {proposal.reason && (
        <p className="text-muted-foreground">
          {t('approval.playbook.why', { reason: proposal.reason })}
        </p>
      )}
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        <Field
          label={t('approval.playbook.description')}
          value={proposal.description}
          was={was('description')}
        />
        <Field
          label={t('approval.playbook.when')}
          value={proposal.when}
          was={was('when')}
        />
      </dl>
      <Collapsible>
        <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-2 hover:underline">
          {proposal.current
            ? t('approval.playbook.showChanges')
            : t('approval.playbook.showSteps')}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <pre
            data-testid="playbook-proposal-body"
            className="mt-2 max-h-72 overflow-auto rounded-md border bg-muted/60 px-3 py-2 text-start font-mono text-xs whitespace-pre-wrap break-words text-foreground"
          >
            {proposal.current
              ? lineDiff(proposal.current.body, proposal.body).map(
                  (line, index) => (
                    <span
                      key={index}
                      data-kind={line.kind}
                      className={cn(
                        'block',
                        line.kind === 'added' && 'bg-success/10 text-success',
                        line.kind === 'removed' &&
                          'bg-destructive/10 text-destructive line-through'
                      )}
                    >
                      {`${line.kind === 'added' ? '+ ' : line.kind === 'removed' ? '- ' : '  '}${line.text}`}
                    </span>
                  )
                )
              : proposal.body}
          </pre>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

function Field({
  label,
  value,
  was,
}: {
  label: string
  value: string
  was: string | undefined
}) {
  const { t } = useTranslation()
  return (
    <>
      <dt className="text-xs font-medium text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words text-foreground">
        {value}
        {was !== undefined && (
          <span className="block text-xs text-muted-foreground line-through">
            {t('approval.playbook.was', { text: was })}
          </span>
        )}
      </dd>
    </>
  )
}
