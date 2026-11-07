import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronDown, Copy, Ellipsis, ShieldCheck } from 'lucide-react'
import { toast } from 'sonner'
import { PilotConsentDialog } from '@/components/agent-console/PilotConsentDialog'
import { ForkDialog } from '@/components/marketplace/ForkDialog'
import { MarketplaceIcon } from '@/components/marketplace/MarketplaceIcon'
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
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { useAgents } from '@/hooks/useAgents'
import type { PilotDefinition } from '@/lib/bindings'
import { uninstallInstalled } from '@/lib/marketplace-actions'
import { useNavigationStore } from '@/store/navigation-store'
import { ConnectorSetup } from './ConnectorSetup'
import { PackConnectorsNotice } from '@/components/connectors/PackConnectorsNotice'
import { AccessList } from './InstallConsentDialog'
import { updateItem } from './library-actions'
import type { LibraryEntry } from './library-entry'
import { displayName } from './library-format'
import { StatusBadge } from './LibraryCard'
import {
  LibraryItemAction,
  type LibraryItemActionProps,
} from './LibraryItemAction'

/**
 * Everything that came off the card (§4.3): the access in full, Setup for
 * connectors, Agents allowed for pilots, a pack's contents, and Details
 * (IDs, ports, keychain entries, usage) collapsed at the end.
 */
export function LibraryDrawer({
  entry,
  onClose,
  onInstall,
  onOpenDrawer,
}: {
  entry: LibraryEntry | null
  onClose: () => void
} & Omit<LibraryItemActionProps, 'entry'>) {
  const { t } = useTranslation()
  return (
    <Sheet open={entry !== null} onOpenChange={open => !open && onClose()}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-[480px]">
        {entry && (
          <DrawerBody
            entry={entry}
            onInstall={onInstall}
            onOpenDrawer={onOpenDrawer}
          />
        )}
        {!entry && (
          <SheetTitle className="sr-only">{t('nav.library')}</SheetTitle>
        )}
      </SheetContent>
    </Sheet>
  )
}

function DrawerBody({
  entry,
  onInstall,
  onOpenDrawer,
}: { entry: LibraryEntry } & Omit<LibraryItemActionProps, 'entry'>) {
  const { t } = useTranslation()
  const { item, source } = entry
  const name = displayName(item, t)

  return (
    <div className="flex flex-col gap-5 p-4">
      <SheetHeader className="p-0">
        <div className="flex items-start gap-3">
          <MarketplaceIcon
            url={null}
            kind={
              item.kind === 'tools'
                ? 'tool'
                : item.kind === 'packs'
                  ? 'pack'
                  : item.kind === 'workflows'
                    ? 'workflow'
                    : 'agent'
            }
            className="size-10 shrink-0"
          />
          <div className="min-w-0 flex-1 space-y-1">
            <SheetTitle>{name}</SheetTitle>
            <SheetDescription>
              {[
                t(`library.type.${item.type}`),
                item.publisher &&
                  t('library.drawer.by', { publisher: item.publisher }),
                item.publisher &&
                  (item.verified
                    ? t('library.install.verified')
                    : t('library.install.unverified')),
                item.version && `v${item.version}`,
              ]
                .filter(Boolean)
                .join(' · ')}
            </SheetDescription>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {item.pill && <StatusBadge pill={item.pill} />}
          <div className="ms-auto flex items-center gap-2">
            <LibraryItemAction
              entry={entry}
              onInstall={onInstall}
              onOpenDrawer={onOpenDrawer}
            />
            <ItemMenu entry={entry} />
          </div>
        </div>
      </SheetHeader>

      {(item.description || item.type === 'workflow') && (
        <p className="text-sm">
          {item.type === 'workflow'
            ? t('library.card.partOf', { pack: item.packName })
            : item.description}
        </p>
      )}

      {item.canDo.length > 0 && item.kind !== 'tools' && (
        <Section title={t('library.drawer.canDo')}>
          <p className="text-sm">{item.canDo.join(' · ')}</p>
        </Section>
      )}

      {(source.type === 'agent' || source.type === 'pack') && (
        <Section title={t('library.drawer.allowed')}>
          <AccessList
            capabilities={
              source.type === 'agent'
                ? source.agent.capabilities
                : source.pack.capabilities
            }
          />
        </Section>
      )}

      {source.type === 'pack' && (
        <PackConnectorsNotice required={source.pack.requiresConnectors} />
      )}

      {source.type === 'builtinTool' && (
        <Section title={t('library.drawer.allowed')}>
          <p className="text-sm">
            {source.entry.definition.requiresApproval
              ? t('library.drawer.toolAsks')
              : t('library.drawer.toolNoAsk')}
          </p>
        </Section>
      )}

      {source.type === 'connector' && (
        <Section title={t('library.drawer.setup')}>
          <ConnectorSetup
            key={JSON.stringify(source.connector.config ?? {})}
            connector={source.connector}
          />
        </Section>
      )}

      {source.type === 'pilot' && (
        <Section title={t('library.drawer.agentsAllowed')}>
          <PilotGrant pilot={source.pilot} />
        </Section>
      )}

      {(source.type === 'pack' || source.type === 'workflow') && (
        <Section title={t('library.drawer.contents')}>
          <ul className="space-y-1 text-sm">
            {source.pack.agents.map(agent => (
              <li key={agent.id}>
                <span className="font-medium">{agent.name}</span>
                {agent.description && (
                  <span className="text-muted-foreground">
                    {' '}
                    · {agent.description}
                  </span>
                )}
              </li>
            ))}
            {source.pack.workflows.map(workflow => (
              <li key={workflow} className="text-muted-foreground">
                {t('library.drawer.workflow', { name: workflow })}
              </li>
            ))}
          </ul>
          {source.type === 'workflow' && (
            <Button
              size="sm"
              variant="outline"
              className="mt-2"
              onClick={() =>
                onOpenDrawer({ ...item, key: `pack:${source.pack.id}` })
              }
            >
              {t('library.drawer.openPack', { pack: source.pack.name })}
            </Button>
          )}
        </Section>
      )}

      <Details entry={entry} />
    </div>
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
    <section className="space-y-2">
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {title}
      </h3>
      {children}
    </section>
  )
}

/** Technical detail lives here only (principle 5) */
function Details({ entry }: { entry: LibraryEntry }) {
  const { t } = useTranslation()
  const { item, source } = entry
  const rows: [string, string][] = [[t('library.details.id'), item.id]]
  if (source.type === 'agent') {
    rows.push([t('library.details.source'), source.agent.registry])
    if (source.agent.commit)
      rows.push([t('library.details.commit'), source.agent.commit])
  }
  if (source.type === 'pack')
    rows.push([t('library.details.source'), source.pack.registry])
  if (source.type === 'builtinTool') {
    rows.push([t('library.details.usage'), String(source.entry.usageCount)])
  }
  if (source.type === 'pilot') {
    rows.push([t('library.details.port'), String(source.pilot.mcpPort)])
    rows.push([t('library.details.install'), source.pilot.installCommand])
  }
  if (source.type === 'connector') {
    for (const secret of source.connector.secrets ?? []) {
      rows.push([t('library.details.keychain'), secret.keyringEntry])
    }
    for (const tool of source.connector.tools ?? []) {
      rows.push([
        t('library.details.tool'),
        `${source.connector.id}__${tool.name}`,
      ])
    }
  }
  return (
    <Collapsible>
      <CollapsibleTrigger className="flex items-center gap-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {t('library.drawer.details')}
        <ChevronDown aria-hidden className="size-3.5" />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          {rows.map(([label, value]) => (
            <div key={`${label}-${value}`} className="contents">
              <dt className="text-muted-foreground">{label}</dt>
              <dd dir="ltr" className="truncate font-mono">
                {value}
              </dd>
            </div>
          ))}
        </dl>
      </CollapsibleContent>
    </Collapsible>
  )
}

/** Check for update, Fork, Uninstall (asks first), Copy ID */
function ItemMenu({ entry }: { entry: LibraryEntry }) {
  const { t } = useTranslation()
  const { item } = entry
  const [forking, setForking] = useState(false)
  const [uninstalling, setUninstalling] = useState(false)
  const installedFromLibrary =
    item.installed && (item.type === 'agent' || item.type === 'pack')

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            size="icon"
            variant="ghost"
            className="size-8"
            aria-label={t('library.drawer.more', { name: item.name })}
          >
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {installedFromLibrary && (
            <DropdownMenuItem
              onSelect={() => void updateItem(item.id, item.name)}
            >
              {t('library.menu.update')}
            </DropdownMenuItem>
          )}
          {installedFromLibrary && item.type === 'agent' && (
            <DropdownMenuItem onSelect={() => setForking(true)}>
              {t('library.menu.fork')}
            </DropdownMenuItem>
          )}
          {installedFromLibrary && (
            <DropdownMenuItem
              className="text-destructive"
              onSelect={() => setUninstalling(true)}
            >
              {t('library.menu.uninstall')}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem
            onSelect={() =>
              void navigator.clipboard
                .writeText(item.id)
                .then(() => toast.success(t('library.menu.copied')))
            }
          >
            <Copy />
            {t('library.menu.copyId')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ForkDialog
        agent={forking ? { id: item.id, name: item.name } : null}
        onOpenChange={open => !open && setForking(false)}
      />
      <AlertDialog open={uninstalling} onOpenChange={setUninstalling}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('library.menu.uninstallTitle', { name: item.name })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {entry.source.type === 'pack'
                ? t('library.menu.uninstallPack', {
                    agents: entry.source.pack.agents
                      .map(agent => agent.name)
                      .join(', '),
                  })
                : t('library.menu.uninstallAgent')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('library.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() =>
                void uninstallInstalled(item.type, item.id, item.name)
              }
            >
              {t('library.menu.uninstallConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

/** Pick an agent and allow it this pilot; the consent dialog asks first */
function PilotGrant({ pilot }: { pilot: PilotDefinition }) {
  const { t } = useTranslation()
  const { data: agents = [] } = useAgents()
  const [agentId, setAgentId] = useState('')
  const [granting, setGranting] = useState<{ id: string; name: string } | null>(
    null
  )
  const agent = agents.find(a => a.id === agentId)

  if (agents.length === 0) {
    return (
      <div className="flex items-center justify-between gap-2 text-sm">
        <p className="text-muted-foreground">{t('library.drawer.noAgents')}</p>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore.getState().navigate({ kind: 'agents' })
          }
        >
          {t('library.drawer.goToAgents')}
        </Button>
      </div>
    )
  }
  return (
    <div className="flex items-center gap-2">
      <NativeSelect
        aria-label={t('pilots.grant.agent', { name: pilot.name })}
        value={agentId}
        onChange={event => setAgentId(event.target.value)}
        className="h-8 text-xs"
      >
        <NativeSelectOption value="">
          {t('pilots.grant.pickAgent')}
        </NativeSelectOption>
        {agents.map(a => (
          <NativeSelectOption key={a.id} value={a.id}>
            {a.name}
          </NativeSelectOption>
        ))}
      </NativeSelect>
      <Button
        size="sm"
        variant="outline"
        disabled={!agent}
        onClick={() => agent && setGranting({ id: agent.id, name: agent.name })}
      >
        <ShieldCheck />
        {t('pilots.grant.allow')}
      </Button>
      <PilotConsentDialog
        agent={granting}
        pilot={granting ? pilot : null}
        onOpenChange={open => !open && setGranting(null)}
      />
    </div>
  )
}
