import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { useConnectorAction } from '@/hooks/useConnectors'
import { usePilotAction } from '@/hooks/usePilots'
import { useSetToolEnabled } from '@/hooks/useTools'
import { installKey, useMarketplaceStore } from '@/store/marketplace-store'
import { useNavigationStore } from '@/store/navigation-store'
import type { LibraryEntry } from './library-entry'
import { toolLabel } from './library-format'
import { connectorAction, type LibraryItem } from './library-model'
import { toastError } from '@/lib/toast-error'

export interface LibraryItemActionProps {
  entry: LibraryEntry
  /** Opens the install-and-allow dialog */
  onInstall: (entry: LibraryEntry) => void
  /** Opens the drawer (Set up, a pack's contents, Update) */
  onOpenDrawer: (item: LibraryItem) => void
}

/**
 * The one primary button a card (or the drawer header) shows for its
 * item, by state (§4.2 footer table): Install, Open, Set up, Start, Stop,
 * Update, or a built-in tool's On/Off switch. Coming soon: nothing.
 */
export function LibraryItemAction({
  entry,
  onInstall,
  onOpenDrawer,
}: LibraryItemActionProps) {
  const { t } = useTranslation()
  const { item, source } = entry
  const busy = useMarketplaceStore(state =>
    item.type === 'agent' || item.type === 'pack'
      ? installKey(item.type, item.id) in state.busy
      : false
  )

  switch (source.type) {
    case 'agent':
    case 'pack': {
      if (busy) {
        return (
          <Button size="sm" disabled>
            <Spinner />
            {t('library.action.installing')}
          </Button>
        )
      }
      if (item.pill === 'updateAvailable') {
        return (
          <Button
            size="sm"
            variant="outline"
            onClick={() => onOpenDrawer(item)}
          >
            {t('library.action.update')}
          </Button>
        )
      }
      if (item.installed) {
        return (
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              source.type === 'agent'
                ? useNavigationStore
                    .getState()
                    .navigate({ kind: 'agent', agentId: item.id })
                : onOpenDrawer(item)
            }
          >
            {t('library.action.open')}
          </Button>
        )
      }
      return (
        <Button
          size="sm"
          disabled={!item.compatible}
          onClick={() => onInstall(entry)}
        >
          {t('library.action.install')}
        </Button>
      )
    }
    case 'workflow':
      return item.installed ? (
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'workflow', workflowId: item.id })
          }
        >
          {t('library.action.open')}
        </Button>
      ) : null
    case 'builtinTool':
      return <ToolSwitch item={item} enabled={source.entry.enabled} />
    case 'connector':
      return (
        <ConnectorButton entry={entry} onSetUp={() => onOpenDrawer(item)} />
      )
    case 'pilot':
      return item.pill === 'notForThisComputer' ? null : (
        <PilotButton pilotId={item.id} name={item.name} pill={item.pill} />
      )
  }
}

/** On needs no confirmation; Off asks first (agents lose the tool) */
function ToolSwitch({
  item,
  enabled,
}: {
  item: LibraryItem
  enabled: boolean
}) {
  const { t } = useTranslation()
  const setEnabled = useSetToolEnabled()
  const [confirming, setConfirming] = useState(false)
  const label = t(`library.tool.${toolLabel(item.name)}`, {
    defaultValue: item.name,
  })

  const apply = (next: boolean) =>
    setEnabled.mutate(
      { toolId: item.id, enabled: next },
      {
        onError: error =>
          toastError(
            error,
            t(next ? 'library.tool.onFailed' : 'library.tool.offFailed', {
              name: label,
            })
          ),
      }
    )

  return (
    <>
      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        {enabled ? t('library.tool.on') : t('library.tool.off')}
        <Switch
          checked={enabled}
          disabled={setEnabled.isPending}
          aria-label={t('library.tool.switch', { name: label })}
          onCheckedChange={next => (next ? apply(true) : setConfirming(true))}
        />
      </label>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('library.tool.offTitle', { name: label })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('library.tool.offDescription', { name: label })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('library.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => apply(false)}>
              {t('library.tool.offConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function ConnectorButton({
  entry,
  onSetUp,
}: {
  entry: LibraryEntry
  onSetUp: () => void
}) {
  const { t } = useTranslation()
  const action = useConnectorAction()
  if (entry.source.type !== 'connector') return null
  const connector = entry.source.connector
  const next = connectorAction(connector)
  if (next === null) return null
  if (next === 'setUp') {
    return (
      <Button size="sm" variant="outline" onClick={onSetUp}>
        {t('library.action.setUp')}
      </Button>
    )
  }
  return (
    <Button
      size="sm"
      variant={next === 'stop' ? 'outline' : 'default'}
      disabled={action.isPending}
      onClick={() =>
        action.mutate(
          { id: connector.id, action: next },
          { onError: error => toastError(error) }
        )
      }
    >
      {action.isPending && <Spinner />}
      {t(`library.action.${next}`)}
    </Button>
  )
}

function PilotButton({
  pilotId,
  name,
  pill,
}: {
  pilotId: string
  name: string
  pill: LibraryItem['pill']
}) {
  const { t } = useTranslation()
  const action = usePilotAction()
  // "Problem" is a running pilot that failed its health check
  const next =
    pill === null
      ? 'install'
      : pill === 'running' || pill === 'problem'
        ? 'stop'
        : 'start'
  return (
    <Button
      size="sm"
      variant={next === 'stop' ? 'outline' : 'default'}
      disabled={action.isPending}
      onClick={() =>
        action.mutate(
          { pilotId, action: next },
          {
            onError: error =>
              toast.error(t(`pilots.error.${next}`, { name }), {
                description:
                  error instanceof Error ? error.message : String(error),
              }),
          }
        )
      }
    >
      {action.isPending && <Spinner />}
      {t(`library.action.${next}`)}
    </Button>
  )
}
