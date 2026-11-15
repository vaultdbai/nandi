import { DEFAULT_AGENT } from '@/lib/bindings'
import type { AgentInfo } from '@/types/agent'

/** The Studio assistant's agent ID (`model::STUDIO_AGENT_ID` in Rust) */
export const STUDIO_AGENT_ID = '@studio'

/**
 * An `AgentInfo` for display only (avatar + name), for identities that
 * aren't agents on disk: the Studio assistant, or agents named in mock data.
 */
export function displayAgent(id: string, name: string): AgentInfo {
  return {
    id,
    name,
    description: '',
    skillPath: '',
    status: 'idle',
    currentTask: null,
    origin: { kind: 'local' },
    kind: 'local',
  }
}

/** Whether this is Nandi, the default agent (`AgentKind::Default` in Rust) */
export function isDefaultAgent(agent: Pick<AgentInfo, 'kind'>): boolean {
  return agent.kind === 'default'
}

/**
 * The person's own and installed agents: every agent but Nandi. Lists,
 * counts and first-run checks use these; pickers show Nandi as a fixed
 * choice above them (`DEFAULT_AGENT`).
 */
export function ownAgents<T extends Pick<AgentInfo, 'kind'>>(
  agents: readonly T[]
): T[] {
  return agents.filter(agent => !isDefaultAgent(agent))
}

/** Nandi's name in a picker: its own, or the product name before it loads */
export function defaultAgentName(
  agents: readonly Pick<AgentInfo, 'id' | 'name'>[]
): string {
  return agents.find(agent => agent.id === DEFAULT_AGENT)?.name || 'Nandi'
}
