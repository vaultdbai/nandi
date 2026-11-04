import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowLeft, ArrowRightLeft, Pencil, Sparkles } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { AmbiguityList } from '@/components/capability/AmbiguityList'
import { CapabilityPolicyForm } from '@/components/capability/CapabilityPolicyForm'
import { EditCapabilityDialog } from '@/components/capability/EditCapabilityDialog'
import { EvidencePanel } from '@/components/capability/EvidencePanel'
import { ProviderBreakdownTable } from '@/components/capability/ProviderBreakdownTable'
import { TransferOwnershipDialog } from '@/components/capability/TransferOwnershipDialog'
import {
  useCapability,
  useCapabilityEvents,
  useCapabilityListings,
} from '@/hooks/useCapabilities'
import { errorMessage } from '@/hooks/useDelegation'
import type { CapabilityDefinition, JsonValue } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

const DETAIL_TABS = [
  'overview',
  'providers',
  'evidence',
  'ambiguities',
  'policy',
] as const
type DetailTab = (typeof DETAIL_TABS)[number]

/**
 * One capability: header (owner, transfer, edit) and the Overview,
 * Providers, Evidence, Ambiguities, and Policy tabs. Only the owner can
 * edit, transfer, change the policy, or promote ambiguities, and only for
 * a definition in this instance's repo.
 */
export function CapabilityDetailView({
  capabilityId,
}: {
  capabilityId: string
}) {
  const { t } = useTranslation()
  useCapabilityEvents()
  const capability = useCapability(capabilityId)
  const listings = useCapabilityListings()
  const [tab, setTab] = useState<DetailTab>('overview')
  const [transferring, setTransferring] = useState(false)
  const [editing, setEditing] = useState(false)

  const listing = listings.data?.find(
    entry => entry.definition.id === capabilityId
  )
  const editable = !!listing?.owned && listing.source === 'local'
  const definition = capability.data
  const back = (
    <Button
      size="icon"
      variant="ghost"
      className="size-7"
      aria-label={t('capabilities.back')}
      onClick={() =>
        useNavigationStore.getState().navigate({
          kind: 'network',
          tab: 'sharing',
          sharing: 'capabilities',
        })
      }
    >
      <ArrowLeft className="rtl:rotate-180" />
    </Button>
  )

  if (capability.isPending) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <ViewHeader title={capabilityId} leading={back} />
        <div className="flex flex-col gap-3 p-6">
          <Skeleton className="h-8 w-64" />
          <Skeleton className="h-40" />
        </div>
      </div>
    )
  }
  if (!definition) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <ViewHeader title={capabilityId} leading={back} />
        <p className="p-6 text-sm text-destructive">
          {t('capabilities.loadError', {
            error: capability.error
              ? errorMessage(capability.error)
              : capabilityId,
          })}
        </p>
      </div>
    )
  }

  const transferable = editable && definition.owner.transferable !== false

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={definition.name || definition.id}
        titleAdornment={
          <Badge variant="outline" className="tabular-nums">
            v{definition.version}
          </Badge>
        }
        subtitle={t('capabilities.card.owner', {
          owner: definition.owner.displayName || definition.owner.id,
        })}
        leading={
          <>
            {back}
            <Sparkles className="size-4 text-muted-foreground" />
          </>
        }
        actions={
          editable && (
            <>
              {transferable && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setTransferring(true)}
                >
                  <ArrowRightLeft />
                  {t('capabilities.transfer.button')}
                </Button>
              )}
              <Button
                size="sm"
                variant="outline"
                onClick={() => setEditing(true)}
              >
                <Pencil />
                {t('capabilities.edit.button')}
              </Button>
            </>
          )
        }
      />
      <Tabs
        value={tab}
        onValueChange={value => setTab(value as DetailTab)}
        className="flex min-h-0 flex-1 flex-col gap-0"
      >
        <div className="border-b px-4 py-2">
          <TabsList>
            {DETAIL_TABS.map(value => (
              <TabsTrigger key={value} value={value}>
                {t(`capabilities.tabs.${value}`)}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="mx-auto w-full max-w-5xl p-6">
            <TabsContent value="overview">
              <Overview definition={definition} />
            </TabsContent>
            <TabsContent value="providers">
              <ProviderBreakdownTable capabilityId={capabilityId} />
            </TabsContent>
            <TabsContent value="evidence">
              <EvidencePanel
                capabilityId={capabilityId}
                canPromote={editable}
              />
            </TabsContent>
            <TabsContent value="ambiguities">
              <AmbiguityList
                capabilityId={capabilityId}
                canPromote={editable}
              />
            </TabsContent>
            <TabsContent value="policy">
              <CapabilityPolicyForm
                key={definition.updatedAt ?? 0}
                definition={definition}
                editable={editable}
              />
            </TabsContent>
          </div>
        </ScrollArea>
      </Tabs>
      {editable && (
        <>
          <TransferOwnershipDialog
            capabilityId={capabilityId}
            open={transferring}
            onOpenChange={setTransferring}
          />
          {editing && (
            <EditCapabilityDialog
              definition={definition}
              open={editing}
              onOpenChange={setEditing}
            />
          )}
        </>
      )}
    </div>
  )
}

/** Description, body, tools, tags, schemas, and ownership history */
function Overview({ definition }: { definition: CapabilityDefinition }) {
  const { t } = useTranslation()
  const required = definition.requiredTools ?? []
  const optional = definition.optionalTools ?? []
  const tags = definition.tags ?? []
  const history = definition.owner.history ?? []

  return (
    <div className="flex flex-col gap-6">
      {definition.description && (
        <p className="text-sm">{definition.description}</p>
      )}
      {definition.body?.trim() && <MarkdownContent content={definition.body} />}
      <OverviewSection title={t('capabilities.overview.requiredTools')}>
        <BadgeList items={required} empty={t('capabilities.overview.none')} />
      </OverviewSection>
      {optional.length > 0 && (
        <OverviewSection title={t('capabilities.overview.optionalTools')}>
          <BadgeList items={optional} empty="" />
        </OverviewSection>
      )}
      <OverviewSection title={t('capabilities.overview.tags')}>
        <BadgeList items={tags} empty={t('capabilities.overview.none')} />
      </OverviewSection>
      <div className="grid gap-6 md:grid-cols-2">
        <OverviewSection title={t('capabilities.overview.inputSchema')}>
          <SchemaBlock schema={definition.schemas?.input ?? null} />
        </OverviewSection>
        <OverviewSection title={t('capabilities.overview.outputSchema')}>
          <SchemaBlock schema={definition.schemas?.output ?? null} />
        </OverviewSection>
      </div>
      {history.length > 0 && (
        <OverviewSection title={t('capabilities.overview.ownership')}>
          <ol className="flex flex-col gap-1 text-sm">
            {history.map(change => (
              <li key={`${change.at}-${change.to}`}>
                {t('capabilities.overview.transferred', {
                  from: change.from,
                  to: change.to,
                  date: new Date(change.at).toLocaleDateString(),
                })}
                {change.reason && (
                  <span className="text-muted-foreground">
                    {' '}
                    — {change.reason}
                  </span>
                )}
              </li>
            ))}
          </ol>
        </OverviewSection>
      )}
    </div>
  )
}

function OverviewSection({
  title,
  children,
}: {
  title: string
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {title}
      </h3>
      {children}
    </section>
  )
}

function BadgeList({ items, empty }: { items: string[]; empty: string }) {
  if (items.length === 0) {
    return <p className="text-sm text-muted-foreground">{empty}</p>
  }
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map(item => (
        <Badge key={item} variant="secondary" className="font-mono">
          {item}
        </Badge>
      ))}
    </div>
  )
}

function SchemaBlock({ schema }: { schema: JsonValue }) {
  const { t } = useTranslation()
  if (schema === null) {
    return (
      <p className="text-sm text-muted-foreground">
        {t('capabilities.overview.noSchema')}
      </p>
    )
  }
  return (
    <pre className="max-h-80 overflow-auto rounded-md border bg-muted/40 p-3 text-xs">
      {JSON.stringify(schema, null, 2)}
    </pre>
  )
}
