import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MoreHorizontal, Plus, Zap } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { fireView, whenText } from '@/components/automations/automations-model'
import { NewTriggerDialog } from '@/components/triggers/NewTriggerDialog'
import { SOURCE_ICONS } from '@/components/triggers/source-icons'
import {
  DeleteTriggerDialog,
  TestTriggerDialog,
} from '@/components/triggers/TriggerDialogs'
import { FireStatusBadge } from '@/components/triggers/trigger-ui'
import {
  useTargetName,
  useTriggerFires,
  useTriggers,
} from '@/hooks/useTriggers'
import { formatRelativeTime } from '@/lib/format-time'
import { setTriggerEnabled } from '@/lib/trigger-actions'
import { SOURCE_TYPES } from '@/lib/trigger-form'
import type { Trigger, TriggerSourceType } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

type Filter = 'all' | TriggerSourceType

/** Every trigger: when it fires, what it runs, its last firing, on/off. */
export function TriggersListView() {
  const { t } = useTranslation()
  const { data: triggers, isLoading, error, refetch } = useTriggers()
  const [filter, setFilter] = useState<Filter>('all')
  const [creating, setCreating] = useState(false)

  const shown = (triggers ?? []).filter(
    trigger => filter === 'all' || trigger.sourceType === filter
  )
  const newButton = (variant: 'default' | 'outline') => (
    <Button size="sm" variant={variant} onClick={() => setCreating(true)}>
      <Plus />
      {t('triggers.new')}
    </Button>
  )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('nav.triggers')}
        subtitle={t('triggers.subtitle')}
        actions={newButton('outline')}
      />
      <NewTriggerDialog open={creating} onOpenChange={setCreating} />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-7xl flex-col gap-4 p-6">
          {isLoading ? (
            <div className="flex flex-col gap-2">
              {[0, 1, 2].map(i => (
                <Skeleton key={i} className="h-12 rounded-md" />
              ))}
            </div>
          ) : error ? (
            <div className="flex items-center gap-3 text-sm text-destructive">
              {t('triggers.loadError')}
              <Button
                size="sm"
                variant="outline"
                onClick={() => void refetch()}
              >
                {t('automations.tryAgain')}
              </Button>
            </div>
          ) : !triggers || triggers.length === 0 ? (
            <Empty className="py-16">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Zap />
                </EmptyMedia>
                <EmptyTitle>{t('triggers.emptyTitle')}</EmptyTitle>
                <EmptyDescription>
                  {t('triggers.emptyDescription')}
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>{newButton('default')}</EmptyContent>
            </Empty>
          ) : (
            <>
              <div
                role="group"
                aria-label={t('triggers.filterLabel')}
                className="flex flex-wrap gap-1.5"
              >
                {(['all', ...SOURCE_TYPES] as Filter[]).map(chip => (
                  <Button
                    key={chip}
                    size="sm"
                    variant={filter === chip ? 'secondary' : 'ghost'}
                    aria-pressed={filter === chip}
                    className="h-7 rounded-full px-3 text-xs"
                    onClick={() => setFilter(chip)}
                  >
                    {chip !== 'all' && SOURCE_ICONS[chip]}
                    {t(
                      chip === 'all'
                        ? 'triggers.filterAll'
                        : `triggers.source.${chip}`
                    )}
                  </Button>
                ))}
              </div>
              <TriggersTable triggers={shown} />
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}

function TriggersTable({ triggers }: { triggers: Trigger[] }) {
  const { t } = useTranslation()

  if (triggers.length === 0) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        {t('triggers.noneOfType')}
      </p>
    )
  }

  return (
    <div className="overflow-x-auto rounded-lg border">
      <table className="w-full text-sm">
        <thead className="bg-muted/40 text-xs text-muted-foreground">
          <tr className="[&_th]:px-3 [&_th]:py-2 [&_th]:text-start [&_th]:font-medium">
            <th>{t('triggers.columns.on')}</th>
            <th>{t('triggers.columns.name')}</th>
            <th>{t('triggers.columns.when')}</th>
            <th>{t('triggers.columns.runs')}</th>
            <th>{t('triggers.columns.lastFired')}</th>
            <th className="sr-only">{t('triggers.columns.actions')}</th>
          </tr>
        </thead>
        <tbody>
          {triggers.map(trigger => (
            <TriggerRow key={trigger.id} trigger={trigger} />
          ))}
        </tbody>
      </table>
    </div>
  )
}

function TriggerRow({ trigger }: { trigger: Trigger }) {
  const { t, i18n } = useTranslation()
  const target = useTargetName(trigger)
  const { data: fires } = useTriggerFires(trigger.id, 1)
  const last = fires?.[0]
  const lastView = last ? fireView(last, trigger) : null
  const [testing, setTesting] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const when = whenText(trigger, t, i18n.language)

  return (
    <tr
      className={cn(
        'border-t hover:bg-accent/40 [&_td]:px-3 [&_td]:py-2',
        !trigger.enabled && 'text-muted-foreground'
      )}
    >
      <td>
        <Switch
          checked={trigger.enabled}
          aria-label={t('triggers.enabledToggle', { name: trigger.name })}
          onCheckedChange={enabled =>
            void setTriggerEnabled(trigger.id, enabled)
          }
        />
      </td>
      <td>
        <button
          type="button"
          className="flex items-center gap-1.5 text-start font-medium hover:underline [&_svg]:size-3.5 [&_svg]:text-muted-foreground"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'trigger', triggerId: trigger.id })
          }
        >
          {SOURCE_ICONS[trigger.sourceType]}
          {trigger.name}
        </button>
      </td>
      <td className="text-xs">
        {when.detail ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span>{when.text}</span>
            </TooltipTrigger>
            <TooltipContent>
              <code dir="ltr">{when.detail}</code>
            </TooltipContent>
          </Tooltip>
        ) : (
          when.text
        )}
      </td>
      <td className="text-xs">
        {t(`triggers.runsTarget.${trigger.targetType}`, { name: target })}
      </td>
      <td className="text-xs whitespace-nowrap">
        {trigger.lastFiredAt === null ? (
          t('triggers.never')
        ) : (
          <span className="flex items-center gap-2">
            {lastView ? (
              <button
                type="button"
                className="hover:underline"
                onClick={() => useNavigationStore.getState().navigate(lastView)}
              >
                {formatRelativeTime(trigger.lastFiredAt, i18n.language)}
              </button>
            ) : (
              formatRelativeTime(trigger.lastFiredAt, i18n.language)
            )}
            {last && <FireStatusBadge fire={last} />}
          </span>
        )}
      </td>
      <td className="text-end">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t('triggers.rowMenu', { name: trigger.name })}
            >
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => setTesting(true)}>
              {t('triggers.test.open')}
            </DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              onSelect={() => setDeleting(true)}
            >
              {t('triggers.deleteMenu')}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <TestTriggerDialog
          trigger={trigger}
          open={testing}
          onOpenChange={setTesting}
        />
        <DeleteTriggerDialog
          trigger={trigger}
          open={deleting}
          onOpenChange={setDeleting}
        />
      </td>
    </tr>
  )
}
