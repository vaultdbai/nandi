import { useTranslation } from 'react-i18next'
import { Plug } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import { useConnectors, type ConnectorDetails } from '@/hooks/useConnectors'
import { cn } from '@/lib/utils'
import {
  chooseFolder,
  effectiveAccess,
  foldersOf,
  type FolderChoice,
  type SharedFolder,
} from '@/components/library/shared-folders'

export interface ConnectorsPanelProps {
  /** The agent whose connectors these are (for labels and test IDs) */
  agentId?: string
  /** Connector IDs the agent's skill.md declares (`connectors: [...]`) */
  enabled?: readonly string[]
  /**
   * Called with the new list when a toggle flips. The owner writes it to
   * the agent's skill.md (the agent editor's save). Without it, the
   * toggles are read-only.
   */
  onEnabledChange?: (enabled: string[]) => void
  /** The shared folders the agent gets from the filesystem connector */
  folders?: FolderChoice[]
  /** Without it, the folders are read-only */
  onFoldersChange?: (folders: FolderChoice[]) => void
}

/**
 * The connectors an agent uses, and their status. Connectors are off for
 * every agent until its skill.md declares them; this panel flips that per
 * agent and shows the tools each connector would add.
 */
export function ConnectorsPanel({
  agentId,
  enabled = [],
  onEnabledChange,
  folders = [],
  onFoldersChange,
}: ConnectorsPanelProps) {
  const { t } = useTranslation()
  const { data: connectors, error, isPending } = useConnectors()
  const installed = connectors?.filter(c => c.status.installed) ?? []

  const toggle = (id: string, on: boolean) => {
    if (!onEnabledChange) return
    const next = on
      ? [...enabled.filter(e => e !== id), id]
      : enabled.filter(e => e !== id)
    onEnabledChange(next)
  }

  return (
    <div className="space-y-3 p-3">
      <p className="text-xs text-muted-foreground">
        {t('agentConsole.context.connectors.description')}
      </p>
      {error && <p className="text-xs text-destructive">{error.message}</p>}
      {isPending && <Skeleton className="h-16" />}
      {!isPending && !error && installed.length === 0 && (
        <div className="flex flex-col items-center gap-2 py-6 text-center">
          <Plug className="size-5 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">
            {t('connectors.panel.empty')}
          </p>
        </div>
      )}
      <ul className="space-y-2">
        {installed.map(connector => (
          <ConnectorRow
            key={connector.id}
            agentId={agentId}
            connector={connector}
            on={enabled.includes(connector.id)}
            readOnly={!onEnabledChange}
            onToggle={on => toggle(connector.id, on)}
          >
            {connector.id === 'filesystem' &&
              enabled.includes(connector.id) && (
                <FolderPicker
                  agentId={agentId}
                  list={foldersOf(connector.config)}
                  choices={folders}
                  onChange={onFoldersChange}
                />
              )}
          </ConnectorRow>
        ))}
      </ul>
      {!onEnabledChange && installed.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {t('connectors.panel.readOnly')}
        </p>
      )}
    </div>
  )
}

function ConnectorRow({
  agentId,
  connector,
  on,
  readOnly,
  onToggle,
  children,
}: {
  agentId?: string
  connector: ConnectorDetails
  on: boolean
  readOnly: boolean
  onToggle: (on: boolean) => void
  children?: React.ReactNode
}) {
  const { t } = useTranslation()
  const switchId = `connector-${agentId ?? 'agent'}-${connector.id}`
  const tools = connector.tools ?? []
  const healthy = connector.status.healthy
  return (
    <li className="rounded-md border p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden
            className={cn(
              'size-2 shrink-0 rounded-full',
              healthy ? 'bg-success' : 'bg-muted-foreground/30'
            )}
          />
          <Label htmlFor={switchId} className="truncate">
            {connector.name}
          </Label>
          <span className="text-xs text-muted-foreground">
            {healthy
              ? t('agentConsole.context.connectors.connected')
              : t('agentConsole.context.connectors.notConnected')}
          </span>
        </div>
        <Switch
          id={switchId}
          checked={on}
          disabled={readOnly}
          onCheckedChange={onToggle}
          aria-label={t('connectors.panel.toggle', { name: connector.name })}
        />
      </div>
      {tools.length > 0 && (
        <ul className="mt-2 flex flex-wrap gap-1">
          {tools.map(tool => (
            <li key={tool.name}>
              <Badge
                variant={tool.sensitive ? 'outline' : 'secondary'}
                className="font-mono text-[10px]"
                title={tool.description}
              >
                {connector.id}__{tool.name}
              </Badge>
            </li>
          ))}
        </ul>
      )}
      {children}
    </li>
  )
}

/** Which shared folders the agent gets, and whether it may write */
function FolderPicker({
  agentId,
  list,
  choices,
  onChange,
}: {
  agentId?: string
  list: SharedFolder[]
  choices: FolderChoice[]
  onChange?: (folders: FolderChoice[]) => void
}) {
  const { t } = useTranslation()
  if (list.length === 0) {
    return (
      <p className="mt-2 text-xs text-muted-foreground">
        {t('connectors.folders.none')}
      </p>
    )
  }
  return (
    <div className="mt-3 space-y-1.5 border-t pt-2">
      <p className="text-xs font-medium">{t('connectors.folders.title')}</p>
      <ul className="space-y-1">
        {list.map(folder => {
          const choice = choices.find(c => c.id === folder.id)
          const value = choice ? effectiveAccess(folder, choice) : 'off'
          const id = `folder-${agentId ?? 'agent'}-${folder.id}`
          return (
            <li key={folder.id} className="flex items-center gap-2">
              <Label
                htmlFor={id}
                className="min-w-0 flex-1 truncate text-xs"
                title={folder.path}
              >
                <span className="font-mono">{folder.id}</span>
                <span className="ms-2 text-muted-foreground">
                  {folder.path}
                </span>
              </Label>
              <NativeSelect
                id={id}
                className="h-7 py-0 text-xs"
                value={value}
                disabled={!onChange}
                onChange={event =>
                  onChange?.(
                    chooseFolder(
                      choices,
                      folder,
                      event.target.value as 'off' | 'read' | 'write'
                    )
                  )
                }
              >
                <NativeSelectOption value="off">
                  {t('connectors.folders.off')}
                </NativeSelectOption>
                <NativeSelectOption value="read">
                  {t('library.folders.read')}
                </NativeSelectOption>
                <NativeSelectOption
                  value="write"
                  disabled={folder.access !== 'write'}
                >
                  {t('library.folders.write')}
                </NativeSelectOption>
              </NativeSelect>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
