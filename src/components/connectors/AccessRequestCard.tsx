import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, Clock, KeyRound, X } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { useNow } from '@/components/agents/task-utils'
import { approvalClock, minutes } from '@/components/inbox/approval-model'
import { useAgents } from '@/hooks/useAgents'
import { client } from '@/lib/client'
import type { AccessChoice } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import {
  accessChoices,
  accessSentence,
  agentLabel,
  decidedKey,
  DEFAULT_AGENT_SCOPE,
  rawSiteHost,
  type AccessRequest,
} from './access-model'

interface AccessRequestCardProps {
  approvalId: string
  request: AccessRequest
  status: 'pending' | 'approved' | 'rejected' | 'expired'
  createdAt: number
  expiresAt?: number | null
  /** Who's asking, by name; else from the agent id */
  agentName?: string
  /** Shows the answers; a read-only transcript leaves them out */
  interactive: boolean
  className?: string
}

/**
 * An agent asking for access to the person's email, calendar, files or a
 * connector: Allow once, Allow for this agent (saved or this computer
 * only), or Don't allow. A policy that asks every time offers only once.
 * A browser pilot asking to open a site: Allow once, Always allow for the
 * agent (optionally with its subdomains), or Don't allow.
 */
export function AccessRequestCard({
  approvalId,
  request,
  status,
  createdAt,
  expiresAt,
  agentName,
  interactive,
  className,
}: AccessRequestCardProps) {
  const { t } = useTranslation()
  const [submitting, setSubmitting] = useState(false)
  const [scope, setScope] = useState<AccessChoice>(DEFAULT_AGENT_SCOPE)
  const [subdomains, setSubdomains] = useState(false)
  // What this card answered, until the run record carries it
  const [answered, setAnswered] = useState<AccessChoice | null>(null)
  const pending = status === 'pending' && !answered
  const now = useNow(pending, 30_000)
  const clock = approvalClock(now, createdAt, expiresAt ?? null)
  const agents = useAgents()
  const who =
    agentName ??
    agents.data?.find(agent => agent.id === request.agent)?.name ??
    agentLabel(request.agent)
  const sentence = accessSentence(request, who)
  const choices = accessChoices(request)
  const site = request.site
  const rawHost = site ? rawSiteHost(site) : null
  const forAgent = !site && choices.includes('allowSaved')
  const always = site !== null && choices.includes('allowSaved')

  const answer = async (choice: AccessChoice) => {
    setSubmitting(true)
    const result = await client.answerAccessRequest(
      approvalId,
      choice,
      site !== null && subdomains
    )
    if (result.status === 'error') {
      toast.error(t('access.answerFailed'), { description: result.error })
      setSubmitting(false)
      return
    }
    setAnswered(choice)
  }

  const decided =
    status === 'pending'
      ? answered
      : decidedKey({ ...request, choice: request.choice ?? answered }, status)

  return (
    <Card
      data-testid="access-request-card"
      className={cn(
        'gap-3 px-4 py-4 shadow-none',
        pending && 'border-warning/40 bg-warning/5',
        className
      )}
    >
      <div className="flex items-start gap-3">
        <KeyRound
          aria-hidden
          className={cn(
            'mt-0.5 size-4 shrink-0',
            pending ? 'text-warning' : 'text-muted-foreground'
          )}
        />
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t('access.label')}
          </p>
          <p className="text-sm font-medium break-words text-foreground">
            {t(`access.sentence.${sentence.key}`, sentence.values)}
          </p>
          {site && rawHost && (
            <p
              data-testid="access-site-raw"
              className="font-mono text-xs break-all text-muted-foreground"
            >
              {t('access.site.raw', { host: rawHost })}
            </p>
          )}
          {site?.reason && (
            <p className="text-sm break-words text-muted-foreground">
              {t('access.site.reason', { reason: site.reason })}
            </p>
          )}
          {request.details.length > 0 && (
            <ul className="list-disc space-y-0.5 ps-5 text-sm text-muted-foreground">
              {request.details.map(detail => (
                <li key={detail}>{detail}</li>
              ))}
            </ul>
          )}
          {request.askEveryTime && (
            <p className="text-xs text-muted-foreground">
              {t('access.askEveryTime')}
            </p>
          )}
          {pending && (
            <p className="flex items-center gap-1 text-xs text-muted-foreground">
              <Clock aria-hidden className="size-3" />
              {clock.kind === 'expiresIn'
                ? t('approval.expiresIn', { minutes: minutes(clock.ms) })
                : clock.kind === 'waiting'
                  ? t('approval.waiting', { minutes: minutes(clock.ms) })
                  : t('approval.expiring')}
            </p>
          )}
        </div>
      </div>

      {decided && (
        <p
          data-testid="access-decided"
          className="flex items-center gap-1.5 text-sm text-muted-foreground"
        >
          {decided === 'deny' || decided === 'expired' ? (
            <X aria-hidden className="size-4" />
          ) : (
            <Check aria-hidden className="size-4 text-success" />
          )}
          {site && decided === 'allowSaved'
            ? request.decidedBy
              ? t('access.site.alwaysBy', { who: request.decidedBy })
              : t('access.site.always')
            : request.decidedBy
              ? t(`access.decidedBy.${decided}`, { who: request.decidedBy })
              : t(`access.decided.${decided}`)}
          {site?.subdomains && decided !== 'deny' && decided !== 'expired' && (
            <span>· {t('access.site.withSubdomains')}</span>
          )}
        </p>
      )}

      {pending && interactive && (
        <div className="space-y-3">
          {site && (
            <div className="flex items-center gap-2">
              <Checkbox
                id={`${approvalId}-subdomains`}
                checked={subdomains}
                onCheckedChange={value => setSubdomains(value === true)}
              />
              <Label
                htmlFor={`${approvalId}-subdomains`}
                className="font-normal"
              >
                {t('access.site.subdomains', { site: site.display })}
              </Label>
            </div>
          )}
          {forAgent && (
            <RadioGroup
              value={scope}
              onValueChange={value => setScope(value as AccessChoice)}
              aria-label={t('access.scopeLabel')}
              className="gap-2"
            >
              <div className="flex items-center gap-2">
                <RadioGroupItem id={`${approvalId}-saved`} value="allowSaved" />
                <Label htmlFor={`${approvalId}-saved`} className="font-normal">
                  {t('access.scope.saved')}
                </Label>
              </div>
              <div className="flex items-center gap-2">
                <RadioGroupItem id={`${approvalId}-local`} value="allowLocal" />
                <Label htmlFor={`${approvalId}-local`} className="font-normal">
                  {t('access.scope.local')}
                </Label>
              </div>
            </RadioGroup>
          )}
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={submitting}
              onClick={() => void answer('deny')}
            >
              <X />
              {t('access.deny')}
            </Button>
            <Button
              size="sm"
              variant={forAgent || always ? 'outline' : 'default'}
              disabled={submitting}
              onClick={() => void answer('allowOnce')}
            >
              {t('access.allowOnce')}
            </Button>
            {forAgent && (
              <Button
                size="sm"
                disabled={submitting}
                onClick={() => void answer(scope)}
              >
                <Check />
                {t('access.allowForAgent')}
              </Button>
            )}
            {always && (
              <Button
                size="sm"
                disabled={submitting}
                onClick={() => void answer('allowSaved')}
              >
                <Check />
                {t('access.site.allowAlways', { agent: who })}
              </Button>
            )}
          </div>
        </div>
      )}
    </Card>
  )
}
