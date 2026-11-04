/**
 * A Library card's item together with the data it came from, so the card,
 * its action and the drawer don't each look it up again.
 */
import type {
  AgentManifest,
  PackManifest,
  PilotDefinition,
  PilotStatus,
  ToolEntry,
} from '@/lib/bindings'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import {
  agentItem,
  builtinToolItem,
  connectorItem,
  packItem,
  pilotItem,
  workflowItems,
  type LibraryItem,
} from './library-model'

export type LibrarySource =
  | { type: 'agent'; agent: AgentManifest }
  | { type: 'pack'; pack: PackManifest }
  | { type: 'workflow'; pack: PackManifest }
  | { type: 'builtinTool'; entry: ToolEntry }
  | { type: 'connector'; connector: ConnectorDetails }
  | { type: 'pilot'; pilot: PilotDefinition; status: PilotStatus | undefined }

export interface LibraryEntry {
  item: LibraryItem
  source: LibrarySource
}

/** Every entry the Library can show, from each query's data */
export function libraryEntries(input: {
  agents: AgentManifest[]
  packs: PackManifest[]
  tools: ToolEntry[]
  connectors: ConnectorDetails[]
  pilots: PilotDefinition[]
  pilotStatuses: Record<string, PilotStatus | undefined>
  runsHere: (pilot: PilotDefinition) => boolean
}): LibraryEntry[] {
  return [
    ...input.agents.map(agent => ({
      item: agentItem(agent),
      source: { type: 'agent' as const, agent },
    })),
    ...input.packs.map(pack => ({
      item: packItem(pack),
      source: { type: 'pack' as const, pack },
    })),
    ...input.packs.flatMap(pack =>
      workflowItems([pack]).map(item => ({
        item,
        source: { type: 'workflow' as const, pack },
      }))
    ),
    ...input.tools
      .filter(entry => entry.definition.source.kind === 'builtin')
      .map(entry => ({
        item: builtinToolItem(entry),
        source: { type: 'builtinTool' as const, entry },
      })),
    ...input.connectors.map(connector => ({
      item: connectorItem(connector),
      source: { type: 'connector' as const, connector },
    })),
    ...input.pilots.map(pilot => {
      const status = input.pilotStatuses[pilot.id]
      return {
        item: pilotItem(pilot, status, input.runsHere(pilot)),
        source: { type: 'pilot' as const, pilot, status },
      }
    }),
  ]
}
