import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Gavel, MessagesSquare, Search } from 'lucide-react'
import {
  chatView,
  decisionTime,
  decisionTone,
  decisionWordKey,
  DECISION_KINDS,
  DECISION_WORDS,
  isFiltered,
  monthLabel,
  NO_FILTERS,
  splitDecision,
  toFilter,
  type DecisionFilters,
} from '@/components/decisions/decisions-model'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { errorMessage } from '@/hooks/useDelegation'
import { useDecisionMonths, useDecisions } from '@/hooks/useDecisions'
import type { DecisionRecord } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

const TONE_CLASS = {
  positive: 'text-success',
  negative: 'text-destructive',
  neutral: 'text-muted-foreground',
} as const

/**
 * Home › Decisions: every decision made on the Inbox, from a button or an
 * Ask chat, month by month (`.nandi/decisions/<yyyy-mm>.md` in the repo).
 * Read-only and local: it needs no relay.
 */
export function DecisionsView() {
  const { t, i18n } = useTranslation()
  const months = useDecisionMonths()
  const [picked, setPicked] = useState<string | null>(null)
  const [filters, setFilters] = useState<DecisionFilters>(NO_FILTERS)

  const available = months.data ?? []
  const month = picked && available.includes(picked) ? picked : available[0]
  const decisions = useDecisions(month ?? null, toFilter(filters))
  const set = (patch: Partial<DecisionFilters>) =>
    setFilters(current => ({ ...current, ...patch }))

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('decisions.title')}
        subtitle={t('decisions.subtitle')}
        leading={<Gavel className="size-4 text-muted-foreground" />}
        actions={
          month && (
            <NativeSelect
              aria-label={t('decisions.month')}
              value={month}
              onChange={event => setPicked(event.target.value)}
            >
              {available.map(m => (
                <NativeSelectOption key={m} value={m}>
                  {monthLabel(m, i18n.language)}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          )
        }
      />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-4 p-6">
          {months.isPending ? (
            <Skeleton className="h-24 w-full" />
          ) : months.isError ? (
            <p role="alert" className="text-sm text-destructive">
              {t('decisions.loadError', { error: errorMessage(months.error) })}
            </p>
          ) : !month ? (
            <Empty className="border" data-testid="decisions-empty">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Gavel />
                </EmptyMedia>
                <EmptyTitle>{t('decisions.empty.title')}</EmptyTitle>
                <EmptyDescription>
                  {t('decisions.empty.description')}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <div className="relative min-w-48 flex-1">
                  <Search
                    aria-hidden
                    className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
                  />
                  <Input
                    type="search"
                    value={filters.search}
                    onChange={event => set({ search: event.target.value })}
                    placeholder={t('decisions.search')}
                    aria-label={t('decisions.search')}
                    className="ps-8"
                  />
                </div>
                <NativeSelect
                  aria-label={t('decisions.kind.label')}
                  value={filters.kind}
                  onChange={event => set({ kind: event.target.value })}
                >
                  <NativeSelectOption value="">
                    {t('decisions.kind.all')}
                  </NativeSelectOption>
                  {DECISION_KINDS.map(kind => (
                    <NativeSelectOption key={kind} value={kind}>
                      {t(`decisions.kind.${kind}`)}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
                <NativeSelect
                  aria-label={t('decisions.decision.label')}
                  value={filters.decision}
                  onChange={event => set({ decision: event.target.value })}
                >
                  <NativeSelectOption value="">
                    {t('decisions.decision.all')}
                  </NativeSelectOption>
                  {DECISION_WORDS.map(word => (
                    <NativeSelectOption key={word} value={word}>
                      {t(`decisions.decision.${decisionWordKey(word)}`)}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </div>
              {decisions.isError ? (
                <p role="alert" className="text-sm text-destructive">
                  {t('decisions.loadError', {
                    error: errorMessage(decisions.error),
                  })}
                </p>
              ) : !decisions.data ? (
                <Skeleton className="h-24 w-full" />
              ) : decisions.data.length === 0 ? (
                <Empty className="border" data-testid="decisions-no-match">
                  <EmptyHeader>
                    <EmptyTitle>
                      {t(
                        isFiltered(filters)
                          ? 'decisions.noMatch.title'
                          : 'decisions.noneThisMonth.title'
                      )}
                    </EmptyTitle>
                    <EmptyDescription>
                      {t(
                        isFiltered(filters)
                          ? 'decisions.noMatch.description'
                          : 'decisions.noneThisMonth.description'
                      )}
                    </EmptyDescription>
                  </EmptyHeader>
                  {isFiltered(filters) && (
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => setFilters(NO_FILTERS)}
                    >
                      {t('decisions.clearFilters')}
                    </Button>
                  )}
                </Empty>
              ) : (
                <ul className="flex flex-col divide-y rounded-lg border">
                  {decisions.data.map(record => (
                    <li key={`${record.at}:${record.kind}:${record.itemId}`}>
                      <DecisionRow record={record} />
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}

function DecisionRow({ record }: { record: DecisionRecord }) {
  const { t, i18n } = useTranslation()
  const { word, rest } = splitDecision(record.decision)
  const label = word
    ? [t(`decisions.decision.${decisionWordKey(word)}`), rest]
        .filter(Boolean)
        .join(' ')
    : rest
  const view = chatView(record)
  return (
    <div className="flex flex-col gap-1 px-4 py-3 text-sm">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <Badge
          variant="outline"
          className={cn(TONE_CLASS[decisionTone(record.decision)])}
        >
          {label}
        </Badge>
        <span className="min-w-0 flex-1 truncate font-medium">
          {record.itemLabel}
        </span>
        <time
          className="text-xs text-muted-foreground"
          dateTime={new Date(record.at).toISOString()}
        >
          {decisionTime(record.at, i18n.language)}
        </time>
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>
          {t(`decisions.kind.${record.kind}`, { defaultValue: record.kind })}
        </span>
        {record.decidedBy && (
          <span>{t('decisions.by', { name: record.decidedBy })}</span>
        )}
        {record.chatId &&
          (view ? (
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={() => useNavigationStore.getState().navigate(view)}
            >
              <MessagesSquare />
              {t('decisions.fromChat')}
            </Button>
          ) : (
            <span className="flex items-center gap-1">
              <MessagesSquare className="size-3" aria-hidden />
              {t('decisions.fromChat')}
              <code className="font-mono">{record.chatId}</code>
            </span>
          ))}
      </div>
      {record.reason && (
        <p className="text-sm whitespace-pre-wrap text-muted-foreground">
          {record.reason}
        </p>
      )}
    </div>
  )
}
