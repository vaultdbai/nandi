import { useTranslation } from 'react-i18next'
import {
  NativeSelectOptGroup,
  NativeSelectOption,
} from '@/components/ui/native-select'
import { defaultAgentName, ownAgents } from '@/lib/agent-identity'
import { DEFAULT_AGENT } from '@/lib/bindings'
import type { AgentInfo } from '@/types/agent'

/**
 * The agent choices of a `NativeSelect`: Nandi, the default agent, as a
 * fixed choice on top, then the person's own and installed agents under
 * "Agents". Nandi is never one more item in that list. `label` words each
 * choice (default: the agent's name).
 */
export function AgentOptions({
  agents,
  label = agent => agent.name || agent.id,
}: {
  agents: readonly AgentInfo[]
  label?: (agent: Pick<AgentInfo, 'id' | 'name'>) => string
}) {
  const { t } = useTranslation()
  const own = ownAgents(agents)
  return (
    <>
      <NativeSelectOption value={DEFAULT_AGENT}>
        {label({ id: DEFAULT_AGENT, name: defaultAgentName(agents) })}
      </NativeSelectOption>
      {own.length > 0 && (
        <NativeSelectOptGroup label={t('nav.agents')}>
          {own.map(agent => (
            <NativeSelectOption key={agent.id} value={agent.id}>
              {label(agent)}
            </NativeSelectOption>
          ))}
        </NativeSelectOptGroup>
      )}
    </>
  )
}
