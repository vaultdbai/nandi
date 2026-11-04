import { useState } from 'react'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import { Plus } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useAgents } from '@/hooks/useAgents'
import { useCapabilityListings } from '@/hooks/useCapabilities'
import { usePolicies } from '@/hooks/usePolicies'
import { useWorkflows } from '@/hooks/useSnapshots'
import { useTools } from '@/hooks/useTools'
import {
  ARTIFACT_TABS,
  artifactOf,
  useStudioStore,
  type ArtifactTab,
} from '@/store/studio-store'
import { ArtifactEditor } from './ArtifactEditor'

/**
 * Studio's right pane: Capability, Agent, Workflow, Policy, and Tool tabs,
 * each with a picker and the editor (tools are a read-only catalog)
 */
export function ArtifactTabs() {
  const { t } = useTranslation()
  const tab = useStudioStore(state => state.tab)

  return (
    <Tabs
      value={tab}
      onValueChange={value =>
        useStudioStore.getState().setTab(value as ArtifactTab)
      }
      className="flex h-full min-h-0 flex-col gap-0"
    >
      <TabsList className="m-3 mb-0 shrink-0 self-start">
        {ARTIFACT_TABS.map(kind => (
          <TabsTrigger key={kind} value={kind}>
            {t(`studio.tab.${kind}`)}
          </TabsTrigger>
        ))}
      </TabsList>
      {ARTIFACT_TABS.map(kind => (
        <TabsContent
          key={kind}
          value={kind}
          className="flex min-h-0 flex-1 flex-col"
        >
          {kind === 'tool' ? <ToolCatalog /> : <ArtifactPane tab={kind} />}
        </TabsContent>
      ))}
    </Tabs>
  )
}

/** One entry in a tab's picker; `broken` files still open, to be fixed */
interface PickerOption {
  id: string
  label: string
  broken?: boolean
}

/** What a tab can open, and whether new IDs are allowed */
function useOptions(tab: ArtifactTab): {
  options: PickerOption[]
  creatable: boolean
} {
  const { t } = useTranslation()
  const { data: capabilities = [] } = useCapabilityListings()
  const { data: agents = [] } = useAgents()
  const { data: workflows = [] } = useWorkflows()
  const { data: policies = [] } = usePolicies()
  switch (tab) {
    case 'capability':
      return {
        // Only this repo's files can be edited here
        options: capabilities
          .filter(listing => listing.source === 'local')
          .map(listing => ({
            id: listing.definition.id,
            label: listing.definition.name,
          })),
        creatable: true,
      }
    case 'agent':
      return {
        options: agents.map(a => ({ id: a.id, label: a.name })),
        creatable: false,
      }
    case 'workflow':
      return {
        options: workflows.map(w => ({ id: w.id, label: w.name })),
        creatable: false,
      }
    case 'policy':
      return {
        // NANDI.md first, then every `.nandi/policies/<id>.md`
        options: [
          { id: 'gateway', label: t('ask.artifact.gateway') },
          ...policies.map(p => ({ id: p.id, label: p.name, broken: !p.valid })),
        ],
        creatable: true,
      }
    case 'tool':
      return { options: [], creatable: false }
  }
}

/** `Name (id)`, or just the id when they match; broken ones say so */
function optionText({ id, label, broken }: PickerOption, t: TFunction): string {
  const text = label === id ? id : `${label} (${id})`
  return broken ? t('studio.option.hasErrors', { label: text }) : text
}

function ArtifactPane({ tab }: { tab: ArtifactTab }) {
  const { t } = useTranslation()
  const selected = useStudioStore(state => state.selected[tab])
  const { options, creatable } = useOptions(tab)
  const [newId, setNewId] = useState('')
  const artifact = selected ? artifactOf(tab, selected) : null
  const known = options.some(option => option.id === selected)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b p-3">
        <NativeSelect
          aria-label={t('studio.open', { kind: t(`studio.tab.${tab}`) })}
          value={known ? selected : ''}
          onChange={event =>
            useStudioStore.getState().select(tab, event.target.value || null)
          }
        >
          <NativeSelectOption value="">{t('studio.choose')}</NativeSelectOption>
          {options.map(option => (
            <NativeSelectOption key={option.id} value={option.id}>
              {optionText(option, t)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        {creatable && (
          <form
            className="flex items-center gap-1.5"
            onSubmit={event => {
              event.preventDefault()
              const id = newId.trim()
              if (!id) return
              useStudioStore.getState().select(tab, id)
              setNewId('')
            }}
          >
            <Input
              value={newId}
              onChange={event => setNewId(event.target.value)}
              placeholder={t('studio.newId')}
              aria-label={t('studio.newId')}
              className="h-9 w-44"
            />
            <Button
              type="submit"
              size="sm"
              variant="outline"
              disabled={!newId.trim()}
            >
              <Plus />
              {t('studio.new')}
            </Button>
          </form>
        )}
      </div>
      {artifact ? (
        <ArtifactEditor
          key={`${artifact.kind}:${selected}`}
          artifact={artifact}
        />
      ) : (
        <p className="p-6 text-sm text-muted-foreground">
          {t('studio.nothingOpen')}
        </p>
      )}
    </div>
  )
}

/** The tool catalog, read-only: tools come from connectors and pilots */
function ToolCatalog() {
  const { t } = useTranslation()
  const { data: tools = [] } = useTools()
  return (
    <ScrollArea className="min-h-0 flex-1">
      <p className="p-3 pb-0 text-xs text-muted-foreground">
        {t('studio.toolsReadOnly')}
      </p>
      <ul aria-label={t('studio.tab.tool')} className="flex flex-col gap-1 p-3">
        {tools.map(tool => (
          <li
            key={tool.definition.id}
            className="flex items-start gap-2 rounded-md border px-3 py-2"
          >
            <span className="min-w-0 flex-1">
              <span className="block text-sm font-medium">
                {tool.definition.name}
              </span>
              <span className="block truncate text-xs text-muted-foreground">
                {tool.definition.description}
              </span>
            </span>
            {!tool.enabled && (
              <Badge variant="outline">{t('studio.toolDisabled')}</Badge>
            )}
          </li>
        ))}
      </ul>
    </ScrollArea>
  )
}
