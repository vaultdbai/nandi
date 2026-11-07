import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { toast } from 'sonner'
import { commands } from '@/lib/tauri-bindings'
import type {
  DefaultModelStatus,
  GatewayStatus,
  IdentityStatus,
  RelayStatus,
} from '@/lib/bindings'
import { packManifest } from '@/test/fixtures'
import { emitMockEvent } from '@/test/mock-events'
import { useNavigationStore } from '@/store/navigation-store'
import { OnboardingView } from './OnboardingView'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const fail = (error: string) =>
  vi.fn().mockResolvedValue({ status: 'error', error })

const fresh: IdentityStatus = {
  onboarded: false,
  displayName: null,
  userSlug: null,
  branch: null,
  onboardingStep: null,
}

const noModel: DefaultModelStatus = {
  defaultModel: 'claude:claude-sonnet-5',
  effectiveModel: null,
  claudeSignIn: null,
  trial: null,
}
const usable: DefaultModelStatus = {
  defaultModel: 'claude:claude-sonnet-5',
  effectiveModel: 'claude:claude-sonnet-5',
  claudeSignIn: null,
  trial: null,
}

/** The relay this (test) build comes with */
const BUILT_IN_RELAY = 'wss://relay.example.org/relay'

const relayStatus = (
  relayUrl: string | null,
  relayUrlSource: RelayStatus['relayUrlSource'] = relayUrl ? 'config' : null,
  builtInUrl: string | null = BUILT_IN_RELAY
): RelayStatus => ({
  connected: relayUrl !== null,
  state: relayUrl ? 'connected' : 'disconnected',
  reason: null,
  instanceId: 'i-me',
  relayUrl,
  relayUrlSource,
  builtInUrl,
  agentsOnline: 0,
  instancesOnline: 0,
  lastConnectedAt: null,
  lastHeartbeatAt: null,
})

const gatewayStatus = {
  enabled: true,
  registered: true,
  discoverable: true,
} as unknown as GatewayStatus

const mailbox = (provider: 'outlook' | 'gmail') => ({
  provider,
  available: true,
  connected: false,
  accountName: null,
  accountEmail: null,
  state: 'notConnected' as const,
  error: null,
  lastCheckedAt: null,
  lastFound: 0,
  lastRunId: null,
  backfill: { phase: 'off' as const, backTo: null, read: 0 },
})

const emailStatus = {
  providers: [mailbox('outlook'), mailbox('gmail')],
  intervalMinutes: 30,
  watch: [],
  nextCheckAt: null,
  checking: false,
  backfillRange: 'everything' as const,
}

const research = packManifest('research-pack', 'Research Pack', {
  suggestedAsks: ['Summarize this paper'],
})
const productivity = packManifest('productivity-pack', 'Productivity Pack')

const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

type User = ReturnType<typeof userEvent.setup>

async function fillName(user: User) {
  await user.type(await screen.findByLabelText('Your name'), 'Ada Lovelace')
  await user.click(screen.getByRole('button', { name: 'Continue' }))
  await screen.findByText('Connect a model')
}

async function toEmail(user: User) {
  await fillName(user)
  await user.click(await screen.findByRole('button', { name: 'Continue' }))
  await screen.findByText('Connect your email (optional)')
}

async function toNetwork(user: User) {
  await toEmail(user)
  await user.click(await screen.findByRole('button', { name: 'Skip for now' }))
  await screen.findByText('Work with other people? (optional)')
}

async function toPacks(user: User) {
  await toNetwork(user)
  await user.click(screen.getByRole('button', { name: 'Continue' }))
  await screen.findByText('Pick some agents to start with')
}

describe('OnboardingView', () => {
  beforeEach(() => {
    vi.mocked(toast.success).mockClear()
    vi.mocked(toast.warning).mockClear()
    vi.mocked(toast.error).mockClear()
    Object.assign(commands, {
      getIdentityStatus: ok(fresh),
      setDisplayName: ok(null),
      setUserSlug: ok(null),
      setOnboardingStep: ok(null),
      getLlmKeyStatus: ok([]),
      getDefaultModel: ok(usable),
      setDefaultModel: ok(usable),
      setLlmApiKey: ok(null),
      testLlmKey: ok({ ok: true, latencyMs: 5, failure: null, detail: null }),
      detectOllama: ok({ running: false, models: [] }),
      setRelayUrl: ok(null),
      joinRelay: ok({ kind: 'joined', relay: 'ws://relay.team.test/relay' }),
      getRelayStatus: ok(relayStatus(null)),
      makeGatewayReachable: ok(gatewayStatus),
      getEmailStatus: ok(emailStatus),
      setEmailSettings: ok(emailStatus),
      checkEmailNow: ok(emailStatus),
      listMarketplacePacks: ok([research, productivity]),
      installPack: ok(null),
    })
    useNavigationStore.setState({
      current: { kind: 'onboarding' },
      history: [],
    })
  })

  describe('name', () => {
    it('derives the handle from the name and hides it behind Customize', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)

      await user.type(await screen.findByLabelText('Your name'), 'Ada Lovelace')
      expect(screen.getByTestId('handle-preview')).toHaveTextContent(
        'Your handle: @ada-lovelace'
      )
      expect(screen.queryByLabelText('Handle')).toBeNull()
      // No jargon on the first step
      expect(document.body.textContent).not.toMatch(/slug|branch/i)

      await user.click(screen.getByRole('button', { name: 'Customize' }))
      const handle = screen.getByLabelText('Handle')
      expect(handle).toHaveValue('ada-lovelace')
      await user.clear(handle)
      await user.type(handle, 'ada')
      await user.type(screen.getByLabelText('Your name'), ' Byron')
      expect(handle).toHaveValue('ada')
    })

    it('validates, then saves the step, the name, and the handle in order', async () => {
      const user = userEvent.setup()
      const order: string[] = []
      const track = (what: string) =>
        vi.fn(async () => {
          order.push(what)
          return { status: 'ok', data: null }
        })
      Object.assign(commands, {
        setOnboardingStep: track('step'),
        setDisplayName: track('name'),
        setUserSlug: track('handle'),
      })
      render(<OnboardingView />)

      await screen.findByLabelText('Your name')
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      expect(screen.getByText('Enter your name.')).toBeInTheDocument()
      expect(order).toEqual([])

      await fillName(user)
      expect(order).toEqual(['step', 'name', 'handle'])
      expect(mocks().setOnboardingStep).toHaveBeenCalledWith('model')
      expect(mocks().setDisplayName).toHaveBeenCalledWith('Ada Lovelace')
      expect(mocks().setUserSlug).toHaveBeenCalledWith('ada-lovelace')
    })

    it('stays open when the identity reloads as the name is saved', async () => {
      const user = userEvent.setup()
      const onComplete = vi.fn()
      // The core's side: an earlier setup finished, then the name was cleared
      let stored: IdentityStatus = { ...fresh, onboardingStep: 'done' }
      Object.assign(commands, {
        getIdentityStatus: vi.fn(async () => ({ status: 'ok', data: stored })),
        setOnboardingStep: vi.fn(
          async (step: IdentityStatus['onboardingStep']) => {
            stored = { ...stored, onboardingStep: step }
            return { status: 'ok', data: null }
          }
        ),
        setDisplayName: vi.fn(async (displayName: string) => {
          stored = { ...stored, onboarded: true, displayName }
          // Saving the name tells the app the identity changed, and the
          // reload lands before the next write
          emitMockEvent('identity://changed', null)
          await waitFor(() =>
            expect(screen.queryByLabelText('Your name')).toBeNull()
          )
          return { status: 'ok', data: null }
        }),
      })
      render(<OnboardingView onComplete={onComplete} />)

      await fillName(user)
      expect(stored.onboardingStep).toBe('model')
      expect(onComplete).not.toHaveBeenCalled()
      expect(mocks().setUserSlug).toHaveBeenCalledWith('ada-lovelace')
    })

    it('stays on the name step when saving fails', async () => {
      const user = userEvent.setup()
      Object.assign(commands, { setUserSlug: fail('slug taken') })
      render(<OnboardingView />)

      await user.type(await screen.findByLabelText('Your name'), 'Ada')
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await waitFor(() =>
        expect(toast.error).toHaveBeenCalledWith(
          "Couldn't save: Something went wrong. Open Details for what happened.",
          expect.objectContaining({ action: expect.anything() })
        )
      )
      expect(screen.queryByText('Connect a model')).toBeNull()
    })

    it('shows five steps', async () => {
      render(<OnboardingView />)
      const steps = await screen.findByRole('list', { name: 'Setup steps' })
      expect(steps).toHaveTextContent(/Name.*Model.*Email.*Network.*Agents/)
      expect(steps.querySelector('[aria-current="step"]')).toHaveTextContent(
        'Name'
      )
    })
  })

  describe('model', () => {
    it('uses a key already in the keychain in one click', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getLlmKeyStatus: ok([
          { provider: 'anthropic', configured: true },
          { provider: 'openAi', configured: false },
          { provider: 'deepSeek', configured: false },
        ]),
      })
      render(<OnboardingView />)
      await fillName(user)

      expect(
        await screen.findByText('Found a key for Anthropic on this computer.')
      ).toBeInTheDocument()
      await user.click(
        screen.getByRole('button', { name: 'Use Anthropic (Claude Sonnet 5)' })
      )
      expect(mocks().setDefaultModel).toHaveBeenCalledWith(
        'claude:claude-sonnet-5'
      )
      expect(
        await screen.findByText('Connect your email (optional)')
      ).toBeInTheDocument()
      expect(mocks().setOnboardingStep).toHaveBeenLastCalledWith('email')
    })

    it('tests a typed key before storing it', async () => {
      const user = userEvent.setup()
      Object.assign(commands, { getDefaultModel: ok(noModel) })
      render(<OnboardingView />)
      await fillName(user)

      expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled()
      await user.click(screen.getByRole('radio', { name: /Anthropic/ }))
      await user.type(screen.getByLabelText('Anthropic API key'), 'sk-ant-1')
      await user.click(screen.getByRole('button', { name: 'Test' }))

      expect(
        await screen.findByText('Works. Checked in 5 ms.')
      ).toBeInTheDocument()
      expect(mocks().testLlmKey).toHaveBeenCalledWith('anthropic', 'sk-ant-1')
      expect(mocks().setLlmApiKey).toHaveBeenCalledWith('anthropic', 'sk-ant-1')
      await waitFor(() =>
        expect(mocks().setDefaultModel).toHaveBeenCalledWith(
          'claude:claude-sonnet-5'
        )
      )
    })

    it('never stores a rejected key', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getDefaultModel: ok(noModel),
        testLlmKey: ok({
          ok: false,
          latencyMs: 40,
          failure: 'invalidKey',
          detail: 'HTTP 401: invalid x-api-key',
        }),
      })
      render(<OnboardingView />)
      await fillName(user)

      await user.click(screen.getByRole('radio', { name: /OpenAI/ }))
      await user.type(screen.getByLabelText('OpenAI API key'), 'sk-wrong')
      await user.click(screen.getByRole('button', { name: 'Test' }))

      expect(
        await screen.findByText('The key was rejected.')
      ).toBeInTheDocument()
      expect(mocks().setLlmApiKey).not.toHaveBeenCalled()
      expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled()
    })

    it('says when Ollama is not running', async () => {
      const user = userEvent.setup()
      Object.assign(commands, { getDefaultModel: ok(noModel) })
      render(<OnboardingView />)
      await fillName(user)

      await user.click(screen.getByRole('radio', { name: /Ollama/ }))
      expect(screen.getByText(/Ollama isn't running/)).toBeInTheDocument()
      await user.click(screen.getByRole('button', { name: 'Check again' }))
      expect(mocks().detectOllama).toHaveBeenCalledTimes(2)
    })

    it('offers an installed Ollama model', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getDefaultModel: ok(noModel),
        detectOllama: ok({ running: true, models: ['llama3.2:latest'] }),
      })
      render(<OnboardingView />)
      await fillName(user)

      await user.click(screen.getByRole('radio', { name: /Ollama/ }))
      await user.click(screen.getByRole('button', { name: 'Use Ollama' }))
      expect(mocks().setDefaultModel).toHaveBeenCalledWith('ollama:llama3.2')
    })

    it('asks twice before skipping without a model', async () => {
      const user = userEvent.setup()
      Object.assign(commands, { getDefaultModel: ok(noModel) })
      render(<OnboardingView />)
      await fillName(user)

      await user.click(screen.getByRole('button', { name: 'Skip for now' }))
      expect(screen.getByRole('alert')).toHaveTextContent(
        "Without a model, agents can't run"
      )
      await user.click(screen.getByRole('button', { name: 'Go back' }))
      expect(screen.queryByRole('alert')).toBeNull()

      await user.click(screen.getByRole('button', { name: 'Skip for now' }))
      await user.click(screen.getByRole('button', { name: 'Skip anyway' }))
      expect(
        await screen.findByText('Connect your email (optional)')
      ).toBeInTheDocument()
    })
  })

  describe('email', () => {
    it('offers both mailboxes, checks every 30 minutes, and can be skipped', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toEmail(user)
      expect(screen.getByLabelText('Outlook')).toBeEnabled()
      expect(screen.getByLabelText('Gmail')).toBeEnabled()
      expect(screen.getByLabelText('30 minutes')).toBeChecked()
      await user.click(screen.getByRole('button', { name: 'Skip for now' }))
      expect(
        await screen.findByText('Work with other people? (optional)')
      ).toBeInTheDocument()
      expect(mocks().setEmailSettings).toHaveBeenCalledWith({
        intervalMinutes: 30,
        watch: [],
        backfill: 'everything',
      })
      expect(mocks().setOnboardingStep).toHaveBeenLastCalledWith('network')
    })
  })

  describe('network', () => {
    it('defaults to Not now and leaves the relay alone', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toNetwork(user)

      expect(screen.getByLabelText('Not now')).toBeChecked()
      expect(
        screen.queryByLabelText('Let others send requests to my Nandi')
      ).toBeNull()
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().setRelayUrl).not.toHaveBeenCalled()
      expect(mocks().joinRelay).not.toHaveBeenCalled()
      expect(mocks().makeGatewayReachable).not.toHaveBeenCalled()
    })

    it("joins the build's Nandi network only on Continue", async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toNetwork(user)

      expect(screen.getByText(/at relay\.example\.org/)).toBeInTheDocument()
      await user.click(screen.getByLabelText('Join the Nandi network'))
      // Choosing isn't joining: no seat taken yet
      expect(mocks().joinRelay).not.toHaveBeenCalled()
      expect(
        screen.getByLabelText('Let others send requests to my Nandi')
      ).not.toBeChecked()
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      // The build's own code, which the UI never sees
      expect(mocks().joinRelay).toHaveBeenCalledWith(BUILT_IN_RELAY, null)
      expect(mocks().setRelayUrl).not.toHaveBeenCalled()
    })

    it('publishes the gateway once when "Let others send requests" is ticked', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toNetwork(user)

      await user.click(screen.getByLabelText('Join the Nandi network'))
      await user.click(
        screen.getByLabelText('Let others send requests to my Nandi')
      )
      expect(screen.getByTestId('reachable-note')).toHaveTextContent(
        'let at least one agent accept requests'
      )
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().joinRelay).toHaveBeenCalledTimes(1)
      expect(mocks().makeGatewayReachable).toHaveBeenCalledTimes(1)
    })

    it('offers no Nandi network when the build has none', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getRelayStatus: ok(relayStatus(null, null, null)),
      })
      render(<OnboardingView />)
      await toNetwork(user)
      expect(screen.queryByLabelText('Join the Nandi network')).toBeNull()
      expect(
        screen.getByLabelText("Join my team's network")
      ).toBeInTheDocument()
    })

    it('keeps the build relay already joined as the Nandi network', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getRelayStatus: ok(
          relayStatus(`${BUILT_IN_RELAY}?token=abc`, 'builtIn')
        ),
      })
      render(<OnboardingView />)
      await toNetwork(user)
      expect(screen.getByLabelText('Join the Nandi network')).toBeChecked()
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().joinRelay).not.toHaveBeenCalled()
      expect(mocks().setRelayUrl).not.toHaveBeenCalled()
    })

    it('leaves a relay joined before alone when the user says Not now', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getRelayStatus: ok(
          relayStatus(`${BUILT_IN_RELAY}?token=abc`, 'builtIn')
        ),
      })
      render(<OnboardingView />)
      await toNetwork(user)
      await user.click(screen.getByLabelText('Not now'))
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().setRelayUrl).not.toHaveBeenCalled()
    })

    it('leaves a network joined on this step when the user then says Not now', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toNetwork(user)
      await user.click(screen.getByLabelText("Join my team's network"))
      await user.type(
        screen.getByLabelText('Network address'),
        'ws://relay.team.test/relay'
      )
      await user.click(screen.getByRole('button', { name: 'Join' }))
      await screen.findByText('Joined relay.team.test. Connecting…')
      await user.click(screen.getByLabelText('Not now'))
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().setRelayUrl).toHaveBeenCalledTimes(1)
      expect(mocks().setRelayUrl).toHaveBeenCalledWith('')
    })

    it('keeps a relay set by NANDI_RELAY_URL without reconnecting', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        getRelayStatus: ok(relayStatus('ws://127.0.0.1:8787/relay', 'env')),
      })
      render(<OnboardingView />)
      await toNetwork(user)

      expect(screen.getByLabelText("Join my team's network")).toBeChecked()
      expect(screen.getByLabelText('Network address')).toHaveValue(
        'ws://127.0.0.1:8787/relay'
      )
      expect(screen.getByTestId('relay-env-note')).toHaveTextContent(
        'NANDI_RELAY_URL'
      )
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().joinRelay).not.toHaveBeenCalled()
      expect(mocks().setRelayUrl).not.toHaveBeenCalled()
    })

    it('says a team network is unreachable and still continues', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        joinRelay: ok({ kind: 'unreachable', detail: 'connection refused' }),
      })
      render(<OnboardingView />)
      await toNetwork(user)

      await user.click(screen.getByLabelText("Join my team's network"))
      const url = screen.getByLabelText('Network address')
      await user.type(url, 'https://nope')
      expect(
        screen.getByText('Enter a ws:// or wss:// address.')
      ).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled()

      await user.clear(url)
      await user.type(url, 'ws://relay.team.test/relay{Enter}')
      expect(
        await screen.findByText(/Couldn't reach a relay at relay\.team\.test/)
      ).toBeInTheDocument()
      expect(screen.getByText('connection refused')).toBeInTheDocument()
      await user.click(screen.getByRole('button', { name: 'Continue anyway' }))
      await screen.findByText('Pick some agents to start with')
      expect(mocks().joinRelay).toHaveBeenCalledTimes(1)
      expect(mocks().joinRelay).toHaveBeenCalledWith(
        'ws://relay.team.test/relay',
        null
      )
    })

    it('joins straight from a typed address on Continue', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toNetwork(user)

      await user.click(screen.getByLabelText("Join my team's network"))
      await user.type(
        screen.getByLabelText('Network address'),
        'ws://relay.team.test/relay'
      )
      await user.click(screen.getByRole('button', { name: 'Continue' }))
      expect(
        await screen.findByText('Pick some agents to start with')
      ).toBeInTheDocument()
      expect(mocks().joinRelay).toHaveBeenCalledWith(
        'ws://relay.team.test/relay',
        null
      )
    })

    it('fills the address and join code from a pasted invite', async () => {
      const user = userEvent.setup()
      render(<OnboardingView />)
      await toNetwork(user)

      await user.click(screen.getByLabelText("Join my team's network"))
      await user.click(screen.getByLabelText('Network address'))
      await user.paste(
        'Join my team on Nandi. Paste this:\n\nwss://relay.acme.test/relay#join=acme-2026'
      )
      expect(screen.getByLabelText('Network address')).toHaveValue(
        'wss://relay.acme.test/relay'
      )
      const code = screen.getByLabelText('Join code')
      expect(code).toHaveAttribute('type', 'password')
      expect(code).toHaveValue('acme-2026')
      await user.click(screen.getByRole('button', { name: 'Join' }))
      await screen.findByText(/^Joined relay\./)
      expect(mocks().joinRelay).toHaveBeenCalledWith(
        'wss://relay.acme.test/relay',
        'acme-2026'
      )
      // Never kept on screen once used
      expect(screen.getByLabelText('Join code')).toHaveValue('')
    })

    it.each([
      [{ kind: 'wrongCode' }, "That join code wasn't accepted"],
      [{ kind: 'codeNeeded' }, 'This relay needs a join code'],
      [
        {
          kind: 'relayFull',
          message: 'This relay is at its free limit of 20 users.',
        },
        'This relay is at its free limit of 20 users.',
      ],
      [{ kind: 'versionMismatch', relay: 3, app: 2 }, 'Update Nandi'],
    ])('says calmly why joining didn’t work: %o', async (outcome, words) => {
      const user = userEvent.setup()
      Object.assign(commands, { joinRelay: ok(outcome) })
      render(<OnboardingView />)
      await toNetwork(user)
      await user.click(screen.getByLabelText("Join my team's network"))
      await user.type(
        screen.getByLabelText('Network address'),
        'wss://relay.acme.test/relay'
      )
      await user.type(screen.getByLabelText('Join code'), 'secret-code')
      await user.click(screen.getByRole('button', { name: 'Join' }))
      expect(await screen.findByRole('alert')).toHaveTextContent(words)
      // The code is never shown
      expect(document.body).not.toHaveTextContent('secret-code')
    })
  })

  describe('packs and finish', () => {
    it('installs the chosen packs and lands on the welcome', async () => {
      const user = userEvent.setup()
      const onComplete = vi.fn()
      render(<OnboardingView onComplete={onComplete} />)
      await toPacks(user)

      await user.click(await screen.findByLabelText('Select Research Pack'))
      await user.click(screen.getByRole('button', { name: 'Install 1 pack' }))

      await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1))
      expect(mocks().installPack).toHaveBeenCalledWith('research-pack')
      expect(mocks().setOnboardingStep).toHaveBeenLastCalledWith('done')
      // The Chief of Staff starts on the mail now, not one interval later
      expect(mocks().checkEmailNow).toHaveBeenCalled()
      expect(useNavigationStore.getState().current).toEqual({
        kind: 'home',
        welcome: { packIds: ['research-pack'] },
      })
      // The welcome replaces the old toast that showed a file path
      expect(toast.success).not.toHaveBeenCalled()
    })

    it('allows at most three packs', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        listMarketplacePacks: ok(
          ['a', 'b', 'c', 'd'].map(id =>
            packManifest(`${id}-pack`, `Pack ${id}`)
          )
        ),
      })
      render(<OnboardingView />)
      await toPacks(user)

      for (const id of ['a', 'b', 'c', 'd']) {
        await user.click(await screen.findByLabelText(`Select Pack ${id}`))
      }
      expect(screen.getByLabelText('Select Pack d')).not.toBeChecked()
      expect(screen.getByText(/Pick up to 3 now/)).toBeInTheDocument()
    })

    it('skipping packs lands on Home', async () => {
      const user = userEvent.setup()
      const onComplete = vi.fn()
      render(<OnboardingView onComplete={onComplete} />)
      await toPacks(user)

      await user.click(await screen.findByRole('button', { name: 'Skip' }))
      await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1))
      expect(mocks().installPack).not.toHaveBeenCalled()
      expect(useNavigationStore.getState().current).toEqual({ kind: 'home' })
    })
  })

  describe('resume', () => {
    it('opens where an interrupted setup stopped', async () => {
      const user = userEvent.setup()
      const onComplete = vi.fn()
      Object.assign(commands, {
        getIdentityStatus: ok({
          ...fresh,
          onboarded: true,
          displayName: 'Ada',
          userSlug: 'ada',
          onboardingStep: 'network',
        }),
      })
      render(<OnboardingView onComplete={onComplete} />)

      expect(
        await screen.findByText('Work with other people? (optional)')
      ).toBeInTheDocument()
      await user.click(
        screen.getByRole('button', { name: 'Skip the rest of setup' })
      )
      await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1))
      expect(mocks().setOnboardingStep).toHaveBeenCalledWith('done')
    })

    it('renders nothing on a home that finished setup', async () => {
      const onComplete = vi.fn()
      Object.assign(commands, {
        getIdentityStatus: ok({
          ...fresh,
          onboarded: true,
          displayName: 'Ada',
          userSlug: 'ada',
          onboardingStep: 'done',
        }),
      })
      render(<OnboardingView onComplete={onComplete} />)
      await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1))
      expect(screen.queryByText('Welcome to Nandi')).toBeNull()
    })

    it('opens Model on a named home with no step recorded', async () => {
      const onComplete = vi.fn()
      Object.assign(commands, {
        getIdentityStatus: ok({
          ...fresh,
          onboarded: true,
          displayName: 'Ada',
          userSlug: 'ada',
        }),
      })
      render(<OnboardingView onComplete={onComplete} />)

      expect(await screen.findByText('Connect a model')).toBeInTheDocument()
      expect(
        screen.getByRole('button', { name: 'Skip the rest of setup' })
      ).toBeInTheDocument()
      expect(onComplete).not.toHaveBeenCalled()
    })
  })
})
