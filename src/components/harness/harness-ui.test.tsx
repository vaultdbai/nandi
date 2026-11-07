import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type {
  BedrockModel,
  HarnessInfo,
  LitellmGatewayStatus as GatewayStatus,
} from '@/lib/bindings'
import { groupBedrockModels } from '@/hooks/useBedrockModels'
import { RuntimesPanel } from '@/components/settings/sections/RuntimesSection'
import { BedrockModelSelector } from './BedrockModelSelector'
import { HarnessCard } from './HarnessCard'
import { LitellmGatewayStatus } from './LitellmGatewayStatus'
import { authSummary, runtimeOptions, runtimeStatus } from './runtime-options'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

function harness(
  id: string,
  overrides: Partial<HarnessInfo> = {},
  health: Partial<HarnessInfo['health']> = {}
): HarnessInfo {
  return {
    id,
    displayName: id === 'claude-code' ? 'Claude Code' : id,
    executable: null,
    configDir: null,
    health: {
      installed: false,
      version: null,
      hostAuthenticated: false,
      containerAuthenticated: false,
      configuredProvider: null,
      auth: null,
      lastCheck: Date.now(),
      error: null,
      ...health,
    },
    isDefault: false,
    defaultModel: null,
    supportedModels: [],
    detectedAt: Date.now(),
    imageAvailable: false,
    ...overrides,
  }
}

function model(
  id: string,
  name: string,
  overrides: Partial<BedrockModel> = {}
): BedrockModel {
  const isAnthropic = id.includes('anthropic')
  return {
    id,
    name,
    provider: isAnthropic ? 'anthropic' : 'meta',
    kind: 'foundation',
    region: 'us-east-1',
    isAnthropic,
    requiresGateway: !isAnthropic,
    streaming: true,
    toolUse: true,
    tested: 'tested',
    notes: null,
    fetchedAt: null,
    ...overrides,
  }
}

const claude = harness(
  'claude-code',
  { executable: '/usr/bin/claude' },
  {
    installed: true,
    version: '2.1.301',
    hostAuthenticated: true,
    containerAuthenticated: true,
    configuredProvider: 'bedrock',
    auth: {
      kind: 'bedrock',
      plan: null,
      organization: null,
      awsProfile: 'acme-dev',
      awsRegion: 'us-east-1',
    },
  }
)
const models = [
  model('us.anthropic.claude-sonnet-5', 'Claude Sonnet 5'),
  model('meta.llama3-3-70b-instruct-v1:0', 'Llama 3.3 70B', {
    tested: 'untested',
  }),
  model('amazon.titan-embed-text-v2:0', 'Titan Embeddings', {
    provider: 'amazon',
    tested: 'unsupported',
  }),
]
const stopped: GatewayStatus = {
  running: false,
  endpoint: null,
  model: null,
  containerId: null,
  startedAt: null,
  error: null,
}

describe('HarnessCard', () => {
  it('shows an installed, signed-in harness with its provider', () => {
    render(<HarnessCard harness={claude} />)
    expect(screen.getByText(/2\.1\.301/)).toBeInTheDocument()
    expect(screen.getByTestId('harness-auth')).toHaveTextContent(
      /aws bedrock \(acme-dev · us-east-1\)/i
    )
    expect(
      screen.getByRole('button', {
        name: /test connection|harness\.card\.test/i,
      })
    ).toBeEnabled()
  })

  it('says what it signs in with and whether that reaches the sandbox', () => {
    const signedIn = harness(
      'claude-code',
      { imageAvailable: true },
      {
        installed: true,
        hostAuthenticated: true,
        auth: {
          kind: 'subscription',
          plan: 'enterprise',
          organization: 'Acme',
          awsProfile: null,
          awsRegion: null,
        },
      }
    )
    render(<HarnessCard harness={signedIn} onAddKey={() => undefined} />)
    expect(screen.getByTestId('harness-auth')).toHaveTextContent(
      /signs in with your account login \(Enterprise · Acme\)/i
    )
    expect(screen.getByTestId('harness-container-auth')).toHaveTextContent(
      /can’t sign in yet/i
    )
    // A key is offered last, as a fallback
    expect(
      screen.getByRole('button', { name: /use an api key instead/i })
    ).toBeInTheDocument()
    expect(screen.getByTestId('runtime-status')).toHaveTextContent(
      /needs sign-in/i
    )
  })

  it('a custom CLI that is neither here nor in the image cannot be tested', () => {
    render(<HarnessCard harness={harness('custom:acme')} />)
    expect(screen.getByTestId('runtime-status')).toHaveTextContent(
      /not found|harness\.status\.notFound/i
    )
    expect(
      screen.getByRole('button', {
        name: /test connection|harness\.card\.test/i,
      })
    ).toBeDisabled()
  })

  it('shows the last test result', () => {
    render(
      <HarnessCard
        harness={claude}
        test={{
          harnessId: 'claude-code',
          ok: false,
          latencyMs: 10,
          version: null,
          configuredProvider: null,
          message: 'failed',
          error: 'not signed in',
        }}
      />
    )
    expect(screen.getByTestId('harness-test-result')).toHaveTextContent(
      /failed|harness\.card\.testFailed/i
    )
  })
})

describe('runtimeStatus', () => {
  it('is ready when the sign-in reaches the sandbox, downloaded or not', () => {
    const signedInHere = harness(
      'claude-code',
      {},
      { installed: true, hostAuthenticated: true }
    )
    expect(runtimeStatus(signedInHere)).toBe('needsSignIn')
    // The sandbox downloads on first use: not having it yet is no problem
    expect(runtimeStatus(claude)).toBe('ready')
    expect(runtimeStatus({ ...claude, imageAvailable: true })).toBe('ready')
    expect(runtimeStatus(harness('codex'))).toBe('needsSignIn')
    // Only a custom CLI has to be on this computer
    expect(runtimeStatus(harness('custom:acme'))).toBe('notFound')
    expect(runtimeStatus(harness('custom:acme', {}, { installed: true }))).toBe(
      'needsSignIn'
    )
  })
})

describe('authSummary', () => {
  it('names the sign-in with its plan, organization, or AWS profile', () => {
    expect(
      authSummary({
        kind: 'subscription',
        plan: 'max',
        organization: null,
        awsProfile: null,
        awsRegion: null,
      })
    ).toEqual({
      key: 'harness.auth.subscriptionWith',
      values: { details: 'Max' },
    })
    expect(authSummary(claude.health.auth)).toEqual({
      key: 'harness.auth.bedrockWith',
      values: { details: 'acme-dev · us-east-1' },
    })
    expect(authSummary(null).key).toBe('harness.auth.none')
    expect(
      authSummary({
        kind: 'apiKey',
        plan: null,
        organization: null,
        awsProfile: null,
        awsRegion: null,
      }).key
    ).toBe('harness.auth.apiKey')
  })
})

describe('runtimeOptions', () => {
  it('offers native and every harness', () => {
    const options = runtimeOptions(
      [
        { id: 'claude-code', displayName: 'Claude Code', installed: true },
        { id: 'gemini-cli', displayName: 'Gemini CLI', installed: false },
      ],
      'native'
    )
    expect(options.map(o => o.id)).toEqual([
      'native',
      'claude-code',
      'gemini-cli',
    ])
    expect(options[2]?.installed).toBe(false)
  })

  it('keeps a saved runtime this machine does not know', () => {
    const options = runtimeOptions([], 'custom:mine')
    expect(options.map(o => o.id)).toEqual(['native', 'custom:mine'])
    expect(options[1]?.installed).toBe(false)
  })
})

describe('BedrockModelSelector', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listBedrockModels: ok(models),
      refreshBedrockModels: ok(models),
    })
  })

  it('splits Anthropic and third-party models and drops unsupported ones', () => {
    const groups = groupBedrockModels(models, '')
    expect(groups.anthropic.map(m => m.id)).toEqual([
      'us.anthropic.claude-sonnet-5',
    ])
    expect(groups.thirdParty.map(m => m.id)).toEqual([
      'meta.llama3-3-70b-instruct-v1:0',
    ])
    expect(groupBedrockModels(models, 'LLAMA').anthropic).toEqual([])
    expect(groupBedrockModels(models, 'meta').thirdParty).toHaveLength(1)
    expect(groupBedrockModels(models, 'nothing').thirdParty).toEqual([])
  })

  it('filters as you type and selects a model', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<BedrockModelSelector value={null} onChange={onChange} />)
    const select = screen.getByRole('combobox')
    await waitFor(() =>
      expect(within(select).getAllByRole('option')).toHaveLength(3)
    )
    await user.type(screen.getByRole('textbox'), 'llama')
    expect(within(select).getAllByRole('option')).toHaveLength(2)
    await user.selectOptions(select, 'meta.llama3-3-70b-instruct-v1:0')
    expect(onChange).toHaveBeenCalledWith('meta.llama3-3-70b-instruct-v1:0')
  })
})

describe('LitellmGatewayStatus', () => {
  it('shows a stopped gateway', async () => {
    Object.assign(commands, { getLitellmGatewayStatus: ok(stopped) })
    render(<LitellmGatewayStatus />)
    expect(await screen.findByTestId('litellm-gateway')).toHaveTextContent(
      /stopped|harness\.gateway\.stopped/i
    )
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('shows a running gateway and stops it', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getLitellmGatewayStatus: ok({
        ...stopped,
        running: true,
        model: 'meta.llama3-3-70b-instruct-v1:0',
        endpoint: 'http://127.0.0.1:4000',
      }),
      stopLitellmGateway: ok(stopped),
    })
    render(<LitellmGatewayStatus />)
    expect(await screen.findByText(/127\.0\.0\.1:4000/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /stop/i }))
    await waitFor(() => expect(commands.stopLitellmGateway).toHaveBeenCalled())
  })
})

describe('Settings › Runtimes', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listHarnesses: ok([claude, harness('codex'), harness('gemini-cli')]),
      detectHarnesses: ok([claude]),
      testHarness: ok({
        harnessId: 'claude-code',
        ok: true,
        latencyMs: 900,
        version: '2.1.301',
        configuredProvider: 'bedrock',
        message: 'Opened and closed an ACP session',
        error: null,
      }),
      setHarnessDefault: ok({
        harnessId: 'claude-code',
        model: null,
        updatedAt: Date.now(),
      }),
      listBedrockModels: ok(models),
      getLitellmGatewayStatus: ok(stopped),
      listHarnessSessions: ok([]),
      detectAwsCli: ok({
        installed: false,
        version: null,
        path: null,
        supportsSso: false,
      }),
      listAwsProfiles: ok([]),
      getAwsBedrockApiKeyStatus: ok({ configured: false, source: null }),
    })
  })

  it('renders the detected harnesses and tests one', async () => {
    const user = userEvent.setup()
    render(<RuntimesPanel />)
    const card = await screen.findByTestId('harness-claude-code')
    expect(screen.getByTestId('harness-codex')).toBeInTheDocument()
    await user.click(
      within(card).getByRole('button', {
        name: /test connection|harness\.card\.test/i,
      })
    )
    await waitFor(() =>
      expect(commands.testHarness).toHaveBeenCalledWith('claude-code')
    )
    expect(
      await within(card).findByTestId('harness-test-result')
    ).toBeInTheDocument()
  })

  it('makes a harness the default', async () => {
    const user = userEvent.setup()
    render(<RuntimesPanel />)
    const card = await screen.findByTestId('harness-claude-code')
    await user.click(
      within(card).getByRole('button', {
        name: /make default|harness\.card\.makeDefault/i,
      })
    )
    await waitFor(() =>
      expect(commands.setHarnessDefault).toHaveBeenCalledWith(
        'claude-code',
        null
      )
    )
  })

  it('shows each known runtime at once, then Couldn’t check after 15 s', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      Object.assign(commands, {
        listHarnesses: vi.fn().mockReturnValue(new Promise(() => undefined)),
      })
      render(<RuntimesPanel />)
      // Nandi and the known CLIs render before detection answers
      expect(screen.getByTestId('harness-native')).toBeInTheDocument()
      const claudeCard = screen.getByTestId('harness-pending-claude-code')
      expect(claudeCard).toHaveTextContent(
        /checking|harness\.status\.checking/i
      )
      expect(screen.getByTestId('harness-pending-codex')).toBeInTheDocument()
      expect(
        screen.getByTestId('harness-pending-gemini-cli')
      ).toBeInTheDocument()

      await vi.advanceTimersByTimeAsync(15_000)
      expect(
        await within(claudeCard).findByText(
          /couldn.t check|harness\.status\.couldntCheck/i
        )
      ).toBeInTheDocument()
      expect(
        within(claudeCard).getByRole('button', {
          name: /detect again|harness\.refresh/i,
        })
      ).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })
})
