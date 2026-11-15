import { describe, expect, it } from 'vitest'
import { NativeSelect } from '@/components/ui/native-select'
import { defaultAgentName, displayAgent } from '@/lib/agent-identity'
import { DEFAULT_AGENT } from '@/lib/bindings'
import { render, screen, within } from '@/test/test-utils'
import { AgentOptions } from './AgentOptions'

const nandi = {
  ...displayAgent(DEFAULT_AGENT, 'Nandi'),
  kind: 'default' as const,
}
const writer = displayAgent('writer', 'Writer')

function renderPicker(agents = [nandi, writer]) {
  render(
    <NativeSelect aria-label="Agent" defaultValue="">
      <AgentOptions agents={agents} />
    </NativeSelect>
  )
  return screen.getByRole('combobox', { name: 'Agent' })
}

describe('AgentOptions', () => {
  it('shows Nandi first, apart from the person’s agents', () => {
    const picker = renderPicker()
    const options = within(picker).getAllByRole('option')
    expect(options.map(o => o.textContent)).toEqual(['Nandi', 'Writer'])
    expect(options[0]).toHaveValue(DEFAULT_AGENT)
    const group = within(picker).getByRole('group', { name: 'Agents' })
    expect(within(group).getAllByRole('option')).toHaveLength(1)
  })

  it('offers Nandi even with no agents of the person’s own', () => {
    const picker = renderPicker([])
    expect(within(picker).getAllByRole('option')).toHaveLength(1)
    expect(within(picker).queryByRole('group')).toBeNull()
  })
})

describe('defaultAgentName', () => {
  it('uses Nandi’s own name, or the product name before the list loads', () => {
    expect(defaultAgentName([{ ...nandi, name: 'Nan' }])).toBe('Nan')
    expect(defaultAgentName([])).toBe('Nandi')
  })
})
