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
    builtin: false,
  }
}

/** The built-in Chief of Staff's agent ID */
export const CHIEF_OF_STAFF_AGENT_ID = 'chief-of-staff'
