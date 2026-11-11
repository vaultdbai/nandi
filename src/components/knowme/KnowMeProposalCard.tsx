import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, HeartHandshake, X } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { rawError } from '@/lib/friendly-error'
import { useConfirmKnowMeProposal, useKnowMe } from '@/hooks/useKnowMe'
import type { GroupRef, KnowMeProposal } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import {
  changesSomething,
  groupName,
  withResponsibility,
  without,
} from './knowme-model'

/** What a know-me approval message brings */
interface KnowMeProposalCardProps {
  approvalId: string
  proposal: KnowMeProposal
  status: 'pending' | 'approved' | 'rejected' | 'expired'
  /** Omit for a read-only transcript */
  onDecide?: (
    approvalId: string,
    decision: 'approved' | 'rejected'
  ) => Promise<boolean>
  statusBadge?: React.ReactNode
  className?: string
}

/**
 * A know-me proposal from the Chief of Staff (`knowme_propose`): each change
 * it would make to the card (NANDI.md) and My notes, editable before the
 * person confirms. Work items carry "On my card"; personal ones stay private
 * or go to one group I'm in. Confirm sends the edited proposal
 * (`confirm_know_me_proposal`); Refuse writes nothing.
 */
export function KnowMeProposalCard({
  approvalId,
  proposal: proposed,
  status,
  onDecide,
  statusBadge,
  className,
}: KnowMeProposalCardProps) {
  const { t } = useTranslation()
  const knowMe = useKnowMe()
  const confirm = useConfirmKnowMeProposal()
  const [proposal, setProposal] = useState(proposed)
  const [submitting, setSubmitting] = useState(false)
  const groups = knowMe.data?.groups ?? []
  const editable = status === 'pending' && onDecide !== undefined

  const onConfirm = () => {
    setSubmitting(true)
    confirm.mutate(
      { approvalId, proposal },
      {
        onError: error => {
          toast.error(rawError(error))
          setSubmitting(false)
        },
      }
    )
  }
  const onRefuse = async () => {
    if (!onDecide) return
    setSubmitting(true)
    if (!(await onDecide(approvalId, 'rejected'))) setSubmitting(false)
  }

  return (
    <Card
      data-testid="know-me-proposal"
      className={cn(
        'gap-4 px-4 py-4 shadow-none',
        status === 'pending' && 'border-warning/40 bg-warning/5',
        className
      )}
    >
      <div className="flex items-start gap-3">
        <HeartHandshake
          aria-hidden
          className="mt-0.5 size-4 shrink-0 text-primary"
        />
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t('knowMe.proposal.label')}
          </p>
          <p className="text-sm font-medium break-words" dir="auto">
            {proposal.summary || t('knowMe.proposal.untitled')}
          </p>
          {editable && (
            <p className="text-xs text-muted-foreground">
              {t('knowMe.proposal.hint')}
            </p>
          )}
        </div>
        {statusBadge}
      </div>

      {proposal.responsibilities.length > 0 && (
        <Section title={t('knowMe.proposal.responsibilities')}>
          <ul className="grid gap-3">
            {proposal.responsibilities.map((item, index) => (
              <li
                key={index}
                data-testid="know-me-responsibility"
                className="grid gap-2 rounded-md border bg-background p-2"
              >
                <div className="flex items-center gap-2">
                  {editable ? (
                    <Input
                      aria-label={t('knowMe.proposal.text')}
                      value={item.text}
                      dir="auto"
                      onChange={event =>
                        setProposal(
                          withResponsibility(proposal, index, {
                            text: event.target.value,
                          })
                        )
                      }
                    />
                  ) : (
                    <span className="min-w-0 flex-1 text-sm" dir="auto">
                      {item.text}
                    </span>
                  )}
                  <Badge variant="secondary">
                    {t(`knowMe.kind.${item.kind}`)}
                  </Badge>
                  {editable && (
                    <RemoveButton
                      label={t('knowMe.proposal.remove', { text: item.text })}
                      onClick={() =>
                        setProposal(
                          without(proposal, 'responsibilities', index)
                        )
                      }
                    />
                  )}
                </div>
                {item.kind === 'work' ? (
                  <div className="flex items-center gap-2">
                    <Switch
                      id={`know-me-card-${index}`}
                      checked={item.onCard}
                      disabled={!editable}
                      onCheckedChange={onCard =>
                        setProposal(
                          withResponsibility(proposal, index, { onCard })
                        )
                      }
                    />
                    <Label
                      htmlFor={`know-me-card-${index}`}
                      className="text-xs font-normal"
                    >
                      {item.onCard
                        ? t('knowMe.proposal.onCard')
                        : t('knowMe.proposal.offCard')}
                    </Label>
                  </div>
                ) : (
                  <PersonalSharing
                    groups={groups}
                    value={item.shareWithGroup}
                    editable={editable}
                    onChange={shareWithGroup =>
                      setProposal(
                        withResponsibility(proposal, index, { shareWithGroup })
                      )
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>
      )}

      {(
        [
          ['description', 'knowMe.proposal.description'],
          ['canHelpWith', 'knowMe.proposal.canHelpWith'],
          ['cantHelpWith', 'knowMe.proposal.cantHelpWith'],
        ] as const
      ).map(([field, label]) =>
        proposal[field] === null ? null : (
          <Section key={field} title={t(label)}>
            {editable ? (
              <Textarea
                aria-label={t(label)}
                className="min-h-16 text-sm"
                dir="auto"
                value={proposal[field] ?? ''}
                onChange={event =>
                  setProposal({ ...proposal, [field]: event.target.value })
                }
              />
            ) : (
              <p className="text-sm whitespace-pre-wrap" dir="auto">
                {proposal[field]}
              </p>
            )}
          </Section>
        )
      )}

      {proposal.notes.length > 0 && (
        <Section title={t('knowMe.proposal.notes')}>
          <ul className="grid gap-1.5">
            {proposal.notes.map((note, index) => (
              <li key={index} className="flex items-center gap-2 text-sm">
                <span className="min-w-0 flex-1" dir="auto">
                  {note.text}
                </span>
                <Badge variant="outline">
                  {note.private
                    ? t('knowMe.proposal.private')
                    : t('knowMe.proposal.myNotes')}
                </Badge>
                {editable && (
                  <RemoveButton
                    label={t('knowMe.proposal.remove', { text: note.text })}
                    onClick={() =>
                      setProposal(without(proposal, 'notes', index))
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>
      )}

      {proposal.offer.length > 0 && (
        <Section title={t('knowMe.proposal.offer')}>
          <ul className="flex flex-wrap gap-1.5">
            {proposal.offer.map((name, index) => (
              <li key={name} className="flex items-center gap-1">
                <Badge variant="secondary" dir="ltr" className="font-mono">
                  {name}
                </Badge>
                {editable && (
                  <RemoveButton
                    label={t('knowMe.proposal.remove', { text: name })}
                    onClick={() =>
                      setProposal(without(proposal, 'offer', index))
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>
      )}

      {proposal.suggest.length > 0 && (
        <Section title={t('knowMe.proposal.suggest')}>
          <ul className="grid gap-1.5">
            {proposal.suggest.map((item, index) => (
              <li key={index} className="flex items-start gap-2 text-sm">
                <span className="min-w-0 flex-1" dir="auto">
                  <span className="font-medium">{item.name}</span>
                  {item.why && (
                    <span className="block text-xs text-muted-foreground">
                      {item.why}
                    </span>
                  )}
                </span>
                {editable && (
                  <RemoveButton
                    label={t('knowMe.proposal.remove', { text: item.name })}
                    onClick={() =>
                      setProposal(without(proposal, 'suggest', index))
                    }
                  />
                )}
              </li>
            ))}
          </ul>
          <p className="text-xs text-muted-foreground">
            {t('knowMe.proposal.suggestHint')}
          </p>
        </Section>
      )}

      {editable && (
        <div className="flex justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={submitting}
            onClick={() => void onRefuse()}
          >
            <X />
            {t('knowMe.proposal.refuse')}
          </Button>
          <Button
            size="sm"
            disabled={submitting || !changesSomething(proposal)}
            onClick={onConfirm}
          >
            <Check />
            {t('knowMe.proposal.confirm')}
          </Button>
        </div>
      )}
    </Card>
  )
}

function Section({
  title,
  children,
}: {
  title: string
  children: React.ReactNode
}) {
  return (
    <section className="grid gap-2">
      <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
      {children}
    </section>
  )
}

function RemoveButton({
  label,
  onClick,
}: {
  label: string
  onClick: () => void
}) {
  return (
    <Button
      type="button"
      size="icon"
      variant="ghost"
      className="size-7 shrink-0"
      aria-label={label}
      onClick={onClick}
    >
      <X />
    </Button>
  )
}

/** Private, or one group I'm in */
function PersonalSharing({
  groups,
  value,
  editable,
  onChange,
}: {
  groups: GroupRef[]
  value: string | null
  editable: boolean
  onChange: (group: string | null) => void
}) {
  const { t } = useTranslation()
  const name = groupName(groups, value)
  if (!editable || groups.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        {name
          ? t('knowMe.proposal.sharedWith', { group: name })
          : t('knowMe.proposal.privateOnly')}
      </p>
    )
  }
  return (
    <NativeSelect
      aria-label={t('knowMe.proposal.shareWith')}
      value={name ? (value ?? '') : ''}
      onChange={event => onChange(event.target.value || null)}
    >
      <NativeSelectOption value="">
        {t('knowMe.proposal.privateOnly')}
      </NativeSelectOption>
      {groups.map(group => (
        <NativeSelectOption key={group.id} value={group.id}>
          {t('knowMe.proposal.sharedWith', { group: group.name })}
        </NativeSelectOption>
      ))}
    </NativeSelect>
  )
}
