/**
 * Sharing › What you offer: "Tell Nandi about you" (the know-me chat),
 * "About this Nandi", then each capability your agents serve as Offered,
 * Needs your approval or Hidden, and the limits.
 * Edit changes all of it inline; Advanced opens NANDI.md in Studio and the
 * test chat (network.md §4.4).
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueries } from '@tanstack/react-query'
import { ChevronRight, FileCode, Pencil, Sparkles } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { GatewayChat } from '@/components/gateway/GatewayChat'
import { TellNandiAboutYou } from '@/components/knowme/TellNandiAboutYou'
import { useAgents } from '@/hooks/useAgents'
import { capabilityQueryKeys } from '@/hooks/useCapabilities'
import { errorMessage } from '@/hooks/useDelegation'
import { useGatewayManifest, useSetGatewayManifest } from '@/hooks/useGateway'
import type { GatewayManifest } from '@/lib/bindings'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { useStudioStore } from '@/store/studio-store'
import { AvailabilityEditor } from './AvailabilityEditor'
import { Term } from './NetworkHelp'
import {
  applyOffer,
  buildOffer,
  OFFER_COLUMNS,
  offerChanges,
  parseResponsibilities,
  publicAbout,
  type OfferColumn,
  type OfferedCapability,
  type ServedBy,
} from './offer-model'

/** What each local agent serves, from its card (`undefined`: loading) */
function useServedBy(): ServedBy[] | undefined {
  const agents = useAgents()
  const list = agents.data ?? []
  const cards = useQueries({
    queries: list.map(agent => ({
      queryKey: capabilityQueryKeys.card(agent.id),
      queryFn: async () => unwrapResult(await client.getAgentCard(agent.id)),
      retry: false,
    })),
  })
  if (!agents.data || cards.some(card => card.isPending)) return undefined
  return list.map((agent, index) => ({
    agent: agent.name,
    capabilities: (cards[index]?.data?.capabilities ?? []).map(c => c.name),
  }))
}

export function OfferView() {
  const { t } = useTranslation()
  const manifest = useGatewayManifest()
  const served = useServedBy()
  const [editing, setEditing] = useState(false)

  if (manifest.error) {
    return (
      <p className="text-sm text-destructive">{errorMessage(manifest.error)}</p>
    )
  }
  if (!manifest.data || !served) return <Skeleton className="h-64 w-full" />

  const offer = buildOffer(served, manifest.data.policy)
  return (
    <div className="flex flex-col gap-6" data-testid="offer">
      <AvailabilityEditor manifest={manifest.data} />
      <TellNandiAboutYou />
      {editing ? (
        <OfferEditor
          manifest={manifest.data}
          offer={offer}
          onDone={() => setEditing(false)}
        />
      ) : (
        <OfferSummary
          manifest={manifest.data}
          offer={offer}
          onEdit={() => setEditing(true)}
        />
      )}
      <Collapsible>
        <CollapsibleTrigger asChild>
          <Button variant="ghost" size="sm" className="group gap-1 px-1">
            <ChevronRight className="transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180" />
            {t('network.offer.advanced')}
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent className="mt-2 flex flex-col gap-4">
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            onClick={() => {
              const studio = useStudioStore.getState()
              studio.setTab('policy')
              studio.select('policy', 'gateway')
              useNavigationStore
                .getState()
                .navigate({ kind: 'studio', conversationId: null })
            }}
          >
            <FileCode />
            {t('network.offer.editAsFile')}
          </Button>
          <GatewayChat />
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

function OfferSummary({
  manifest,
  offer,
  onEdit,
}: {
  manifest: GatewayManifest
  offer: OfferedCapability[]
  onEdit: () => void
}) {
  const { t } = useTranslation()
  const about = publicAbout(manifest.description, manifest.body)
  const { policy } = manifest

  return (
    <section className="flex flex-col gap-4" aria-labelledby="offer-title">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="offer-title" className="text-sm font-semibold">
          {t('network.offer.title')}
        </h2>
        <Button size="sm" variant="outline" onClick={onEdit}>
          <Pencil />
          {t('network.offer.edit')}
        </Button>
      </div>
      <div className="grid gap-1">
        <h3 className="text-xs font-medium text-muted-foreground">
          {t('network.offer.about')}
        </h3>
        <p className="line-clamp-6 text-sm whitespace-pre-wrap" dir="auto">
          {about || t('network.offer.noAbout')}
        </p>
      </div>
      <div className="grid gap-1" data-testid="offer-responsibilities">
        <h3 className="text-xs font-medium text-muted-foreground">
          {t('network.offer.responsibilities')}
        </h3>
        {manifest.responsibilities.length > 0 &&
          !manifest.relay.shareResponsibilities && (
            <p className="text-xs text-muted-foreground">
              {t('network.offer.responsibilitiesPrivate')}
            </p>
          )}
        {manifest.responsibilities.length > 0 ? (
          <ul className="flex flex-wrap gap-1.5">
            {manifest.responsibilities.map(item => (
              <li key={item}>
                <Badge variant="secondary" dir="auto">
                  {item}
                </Badge>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t('network.offer.noResponsibilities')}
          </p>
        )}
      </div>
      {offer.length === 0 ? (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Sparkles />
            </EmptyMedia>
            <EmptyTitle>{t('network.offer.nothing')}</EmptyTitle>
            <EmptyDescription>
              {t('network.offer.nothingHint')}
            </EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button
              onClick={() =>
                useNavigationStore.getState().navigate({ kind: 'agents' })
              }
            >
              {t('network.offer.chooseWhat')}
            </Button>
          </EmptyContent>
        </Empty>
      ) : (
        <>
          {policy.expose.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {t('network.offer.everything')}
            </p>
          )}
          <div className="grid gap-4 sm:grid-cols-3">
            {OFFER_COLUMNS.map(column => {
              const items = offer.filter(c => c.column === column)
              return (
                <div
                  key={column}
                  className="grid content-start gap-2"
                  data-testid={`offer-${column}`}
                >
                  <h3 className="text-xs font-medium text-muted-foreground">
                    {column === 'approval' ? (
                      <Term term="needsApproval">
                        {t('network.offer.column.approval', {
                          count: items.length,
                        })}
                      </Term>
                    ) : (
                      t(`network.offer.column.${column}`, {
                        count: items.length,
                      })
                    )}
                  </h3>
                  <ul className="grid gap-1.5">
                    {items.map(item => (
                      <li key={item.name} className="text-sm">
                        <span dir="ltr" className="font-mono text-xs">
                          {item.name}
                        </span>
                        <span className="block text-xs text-muted-foreground">
                          {item.agents.join(', ')}
                        </span>
                      </li>
                    ))}
                  </ul>
                  {column === 'approval' && items.length > 0 && (
                    <p className="text-xs text-muted-foreground">
                      {t('network.offer.approvalHint')}
                    </p>
                  )}
                </div>
              )
            })}
          </div>
        </>
      )}
      <p className="text-xs text-muted-foreground">
        {t('network.offer.limits', {
          max: policy.maxConcurrent,
          rate: policy.rateLimitPerHour,
        })}
      </p>
    </section>
  )
}

/** A whole number ≥ `min`, else `fallback` */
function toCount(value: string, min: number, fallback: number): number {
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed >= min ? parsed : fallback
}

function OfferEditor({
  manifest,
  offer,
  onDone,
}: {
  manifest: GatewayManifest
  offer: OfferedCapability[]
  onDone: () => void
}) {
  const { t } = useTranslation()
  const save = useSetGatewayManifest()
  const [description, setDescription] = useState(manifest.description)
  const [body, setBody] = useState(manifest.body)
  const [responsibilities, setResponsibilities] = useState(
    manifest.responsibilities.join('\n')
  )
  const [shareResponsibilities, setShareResponsibilities] = useState(
    manifest.relay.shareResponsibilities
  )
  const [max, setMax] = useState(manifest.policy.maxConcurrent)
  const [rate, setRate] = useState(manifest.policy.rateLimitPerHour)
  const [choices, setChoices] = useState<Record<string, OfferColumn>>(() =>
    Object.fromEntries(offer.map(c => [c.name, c.column]))
  )

  const submit = () => {
    const changes = offerChanges(offer, choices)
    save.mutate(
      {
        ...manifest,
        description,
        body,
        responsibilities: parseResponsibilities(responsibilities),
        relay: { ...manifest.relay, shareResponsibilities },
        policy: {
          ...applyOffer(manifest.policy, choices),
          maxConcurrent: max,
          rateLimitPerHour: rate,
        },
      },
      {
        onSuccess: () => {
          toast.success(t('network.offer.saved'), {
            description:
              changes
                .map(c =>
                  t(`network.offer.change.${c.column}`, { name: c.name })
                )
                .join(' · ') || undefined,
          })
          onDone()
        },
        onError: error => toast.error(errorMessage(error)),
      }
    )
  }

  return (
    <form
      className="flex flex-col gap-4"
      data-testid="offer-editor"
      onSubmit={event => {
        event.preventDefault()
        submit()
      }}
    >
      <h2 className="text-sm font-semibold">{t('network.offer.title')}</h2>
      <div className="grid gap-2">
        <Label htmlFor="offer-description">
          {t('network.offer.description')}
        </Label>
        <Input
          id="offer-description"
          value={description}
          onChange={event => setDescription(event.target.value)}
        />
      </div>
      <div className="grid gap-2">
        <Label htmlFor="offer-body">{t('network.offer.about')}</Label>
        <Textarea
          id="offer-body"
          className="min-h-32 text-sm"
          value={body}
          onChange={event => setBody(event.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          {t('network.offer.aboutHint')}
        </p>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="offer-responsibilities">
          {t('network.offer.responsibilities')}
        </Label>
        <Textarea
          id="offer-responsibilities"
          className="min-h-20 text-sm"
          dir="auto"
          placeholder={t('network.offer.responsibilitiesPlaceholder')}
          value={responsibilities}
          onChange={event => setResponsibilities(event.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          {t('network.offer.responsibilitiesHint')}
        </p>
        <div className="flex items-center gap-2">
          <Switch
            id="offer-share-responsibilities"
            checked={shareResponsibilities}
            onCheckedChange={setShareResponsibilities}
          />
          <Label htmlFor="offer-share-responsibilities">
            {t('network.offer.shareResponsibilities')}
          </Label>
        </div>
      </div>
      {offer.length > 0 && (
        <ul className="grid gap-2 rounded-md border p-3">
          {offer.map(item => (
            <li
              key={item.name}
              className="flex flex-wrap items-center justify-between gap-2"
            >
              <div className="min-w-0">
                <span dir="ltr" className="font-mono text-xs">
                  {item.name}
                </span>
                <span className="block text-xs text-muted-foreground">
                  {item.agents.join(', ')}
                </span>
              </div>
              <NativeSelect
                aria-label={t('network.offer.choiceFor', { name: item.name })}
                value={choices[item.name]}
                onChange={event =>
                  setChoices(current => ({
                    ...current,
                    [item.name]: event.target.value as OfferColumn,
                  }))
                }
              >
                {OFFER_COLUMNS.map(column => (
                  <NativeSelectOption key={column} value={column}>
                    {t(`network.offer.choice.${column}`)}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </li>
          ))}
        </ul>
      )}
      <div className="grid grid-cols-2 gap-4">
        <div className="grid gap-2">
          <Label htmlFor="offer-max">{t('gateway.editor.maxConcurrent')}</Label>
          <Input
            id="offer-max"
            type="number"
            min={1}
            value={max}
            onChange={event => setMax(toCount(event.target.value, 1, max))}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="offer-rate">{t('gateway.editor.rateLimit')}</Label>
          <Input
            id="offer-rate"
            type="number"
            min={0}
            value={rate}
            onChange={event => setRate(toCount(event.target.value, 0, rate))}
          />
        </div>
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onDone}>
          {t('network.cancel')}
        </Button>
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Spinner />}
          {t('network.offer.save')}
        </Button>
      </div>
    </form>
  )
}
