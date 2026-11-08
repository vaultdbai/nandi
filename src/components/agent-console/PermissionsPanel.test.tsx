import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
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

describe('PermissionsPanel sites and accounts', () => {
  const original: Commands = {}
  let permissions: AgentPilotPermission[]
  const names = [
    'repoLoadEntity',
    'listPilots',
    'agentPilotPermissions',
    'setPilotSecret',
    'addPilotSecret',
    'clearBrowserProfile',
  ]

  const setUp = (readOnly: boolean) => {
    mocked.repoLoadEntity = vi.fn().mockResolvedValue(
      ok({
        ...definition,
        id: 'cos',
        readOnly,
        capabilities: { tools: [], network: [], pilots: ['browser'] },
      })
    )
  }

  beforeEach(() => {
    for (const name of names) original[name] = mocked[name]
    permissions = [
      {
        pilotId: 'browser',
        available: true,
        capabilities: [],
        granted: [],
        secrets: [{ name: 'SITE_PASSWORD', isSet: true }],
        browser: true,
        hasProfile: true,
      },
    ]
    mocked.listPilots = vi.fn().mockResolvedValue(
      ok([
        {
          id: 'browser',
          name: 'Browser',
          platform: 'any',
          installCommand: '',
          startCommand: '',
          mcpPort: 0,
          description: '',
          capabilities: [],
        },
      ])
    )
    mocked.agentPilotPermissions = vi.fn(async () =>
      ok(structuredClone(permissions))
    )
    mocked.setPilotSecret = vi.fn(async () => ok(null))
    mocked.addPilotSecret = vi.fn(
      async (_agent: string, _pilot: string, name: string) => {
        permissions[0]?.secrets?.push({ name, isSet: true })
        return ok(null)
      }
    )
    mocked.clearBrowserProfile = vi.fn(async () => {
      const first = permissions[0]
      if (first) first.hasProfile = false
      return ok(true)
    })
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('adds a site secret by name and clears the value from the form', async () => {
    setUp(false)
    const user = userEvent.setup()
    render(<PermissionsPanel agentId="cos" />)
    const name = await screen.findByRole('textbox', {
      name: 'New secret name',
    })
    const add = screen.getByRole('button', { name: 'Add secret' })
    await user.type(name, 'site password')
    expect(name).toHaveValue('SITE_PASSWORD')
    expect(screen.getByRole('alert')).toHaveTextContent('already has')
    expect(add).toBeDisabled()

    await user.clear(name)
    await user.type(name, 'acme_password')
    const value = screen.getByLabelText('New secret value')
    await user.type(value, 'hunter2')
    await user.click(add)
    expect(mocked.addPilotSecret).toHaveBeenCalledWith(
      'cos',
      'browser',
      'ACME_PASSWORD',
      'hunter2'
    )
    await waitFor(() => expect(value).toHaveValue(''))
    expect(await screen.findByText('ACME_PASSWORD')).toBeInTheDocument()
    expect(screen.queryByDisplayValue('hunter2')).not.toBeInTheDocument()
  })

  it('lets a pack agent set values but not add names', async () => {
    setUp(true)
    render(<PermissionsPanel agentId="cos" />)
    expect(await screen.findByText('SITE_PASSWORD')).toBeInTheDocument()
    expect(screen.getByLabelText('Value of SITE_PASSWORD')).toBeInTheDocument()
    expect(
      screen.queryByRole('textbox', { name: 'New secret name' })
    ).not.toBeInTheDocument()
  })

  it('signs out everywhere after a confirm', async () => {
    setUp(false)
    const user = userEvent.setup()
    render(<PermissionsPanel agentId="cos" />)
    await user.click(
      await screen.findByRole('button', { name: 'Sign out everywhere' })
    )
    expect(mocked.clearBrowserProfile).not.toHaveBeenCalled()
    const dialog = await screen.findByRole('alertdialog')
    await user.click(
      within(dialog).getByRole('button', { name: 'Sign out everywhere' })
    )
    expect(mocked.clearBrowserProfile).toHaveBeenCalledWith('cos', 'browser')
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Sign out everywhere' })
      ).toBeDisabled()
    )
  })
})
