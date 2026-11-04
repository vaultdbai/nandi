import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { AgentPilotPermission } from '@/lib/bindings'
import { PermissionsPanel } from './PermissionsPanel'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const definition = {
  id: 'writer',
  name: 'Writer',
  description: '',
  skillBody: 'Write.',
  instructions: '',
  modelProvider: null,
  modelName: null,
  image: null,
  memoryMb: null,
  cpu: null,
  capabilities: { tools: [], network: [], pilots: ['oswright', 'macpilot'] },
  readOnly: false,
  allowedTools: null,
  requiresApproval: [],
  networkRules: [],
  workflows: [],
  skillMd: '',
  renderedSkillMd: '',
  sourcePath: '/repo/.nandi/agents/writer/skill.md',
}

describe('PermissionsPanel pilots', () => {
  const original: Commands = {}
  let permissions: AgentPilotPermission[]

  beforeEach(() => {
    for (const name of [
      'repoLoadEntity',
      'listPilots',
      'agentPilotPermissions',
      'grantPilotCapability',
      'revokePilotCapability',
    ])
      original[name] = mocked[name]
    permissions = [
      {
        pilotId: 'oswright',
        available: true,
        capabilities: ['app_control', 'browser'],
        granted: ['browser'],
      },
      { pilotId: 'macpilot', available: false, capabilities: [], granted: [] },
    ]
    mocked.repoLoadEntity = vi.fn().mockResolvedValue(ok(definition))
    mocked.listPilots = vi.fn().mockResolvedValue(
      ok([
        {
          id: 'oswright',
          name: 'OSWright',
          platform: 'any',
          installCommand: '',
          startCommand: '',
          mcpPort: 17898,
          description: '',
          capabilities: ['app_control', 'browser'],
        },
      ])
    )
    mocked.agentPilotPermissions = vi.fn(async () => ok(permissions))
    mocked.grantPilotCapability = vi.fn(
      async (_agent: string, _pilot: string, capability: string) => {
        permissions[0]?.granted.push(capability)
        return ok(null)
      }
    )
    mocked.revokePilotCapability = vi.fn(
      async (_agent: string, _pilot: string, capability: string) => {
        const first = permissions[0]
        if (first) {
          first.granted = first.granted.filter(c => c !== capability)
        }
        return ok(null)
      }
    )
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('shows the grants already saved', async () => {
    render(<PermissionsPanel agentId="writer" />)
    expect(
      await screen.findByRole('switch', { name: 'OSWright browser' })
    ).toBeChecked()
    expect(
      screen.getByRole('switch', { name: 'OSWright app_control' })
    ).not.toBeChecked()
    // Declared, but not in this OS's catalog
    expect(screen.getByText('macpilot')).toBeInTheDocument()
    expect(mocked.agentPilotPermissions).toHaveBeenCalledWith('writer')
  })

  it('revokes and grants a capability', async () => {
    const user = userEvent.setup()
    render(<PermissionsPanel agentId="writer" />)
    const browser = await screen.findByRole('switch', {
      name: 'OSWright browser',
    })

    await user.click(browser)
    expect(mocked.revokePilotCapability).toHaveBeenCalledWith(
      'writer',
      'oswright',
      'browser'
    )
    await waitFor(() =>
      expect(
        screen.getByRole('switch', { name: 'OSWright browser' })
      ).not.toBeChecked()
    )

    await user.click(
      screen.getByRole('switch', { name: 'OSWright app_control' })
    )
    expect(mocked.grantPilotCapability).toHaveBeenCalledWith(
      'writer',
      'oswright',
      'app_control'
    )
    await waitFor(() =>
      expect(
        screen.getByRole('switch', { name: 'OSWright app_control' })
      ).toBeChecked()
    )
  })
})
