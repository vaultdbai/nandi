import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { AgentWarningBanner } from '@/components/agent-console/ContainerControls'
import { StepComputer } from '@/components/onboarding/StepComputer'
import { ThisComputerSection } from '@/components/settings/sections/ThisComputerSection'
import { commands } from '@/lib/tauri-bindings'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { MachineChecklist } from './MachineChecklist'
import { MachineHomeItem } from './MachineHomeItem'
import { check, report } from './machine.fixtures'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

/** Windows, Docker Desktop installed but "unable to start" */
const unableToStart = report({
  dockerReady: false,
  ready: false,
  checks: [
    check({ id: 'dockerInstalled', detail: 'Docker Desktop at C:\\Docker' }),
    check({
      id: 'dockerRunning',
      state: 'blocked',
      reason: 'desktopFailed',
      detail: 'Docker Desktop is unable to start',
      actions: [
        { kind: 'startDocker' },
        {
          kind: 'openLink',
          url: 'https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/',
        },
        { kind: 'checkAgain' },
      ],
    }),
    check({ id: 'wsl', detail: 'WSL 3.0.5.0' }),
    check({ id: 'disk', detail: '38.0 GB free' }),
  ],
})

const ready = report({
  checks: [
    check({ id: 'dockerInstalled' }),
    check({ id: 'dockerRunning', detail: 'Docker Engine 29.9.2' }),
  ],
})

describe('MachineChecklist', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    Object.assign(commands, {
      checkMachine: ok(unableToStart),
      startDocker: ok(null),
    })
  })

  it('shows each check, and why and how for the one that isn’t ready', async () => {
    const user = userEvent.setup()
    render(<MachineChecklist report={unableToStart} />)
    const running = screen.getByTestId('machine-dockerRunning')
    expect(running).toHaveAttribute('data-state', 'blocked')
    expect(running).toHaveTextContent('Docker running')
    expect(running).toHaveTextContent('Docker Desktop is unable to start')
    expect(running).toHaveTextContent(
      'Docker has to be running for agents to use files'
    )
    const steps = within(running).getAllByRole('listitem')
    expect(steps).toHaveLength(6)
    expect(steps[1]).toHaveTextContent('wsl --update')
    expect(steps[5]).toHaveTextContent('Reset to factory defaults')

    // Ready rows say so and nothing more
    const installed = screen.getByTestId('machine-dockerInstalled')
    expect(installed).toHaveTextContent('Ready')
    expect(within(installed).queryByRole('button')).toBeNull()

    await user.click(
      within(running).getByRole('button', { name: 'Start Docker' })
    )
    expect(commands.startDocker).toHaveBeenCalled()
    expect(
      within(running).getByRole('button', { name: 'Read how' })
    ).toBeInTheDocument()
    // Check again is the page's button, not each row's
    expect(
      within(running).queryByRole('button', { name: 'Check again' })
    ).toBeNull()
  })

  it('shows copyable commands left to right', () => {
    render(<MachineChecklist report={unableToStart} />)
    const code = screen.getByText('wsl --update')
    expect(code).toHaveAttribute('dir', 'ltr')
    expect(
      screen.getAllByRole('button', { name: 'Copy command' }).length
    ).toBeGreaterThan(0)
  })

  it('Settings › This computer says agents can only talk, and Check again re-runs everything', async () => {
    const user = userEvent.setup()
    render(<ThisComputerSection />)
    expect(
      await screen.findByText(
        "Agents can talk but can't act until Docker is ready. Nothing else waits on this."
      )
    ).toBeInTheDocument()
    expect(commands.checkMachine).toHaveBeenCalledWith(false)
    vi.mocked(commands.checkMachine).mockResolvedValueOnce({
      status: 'ok',
      data: ready,
    })
    await user.click(screen.getByRole('button', { name: 'Check again' }))
    expect(commands.checkMachine).toHaveBeenCalledWith(true)
    expect(
      await screen.findByText('This computer is ready. Agents can act.')
    ).toBeInTheDocument()
  })

  it('the onboarding step is skippable and says what Docker is for', async () => {
    const user = userEvent.setup()
    const onNext = vi.fn()
    const onBack = vi.fn()
    render(<StepComputer onNext={onNext} onBack={onBack} />)
    expect(
      screen.getByText('Without Docker, agents can only talk, not act.')
    ).toBeInTheDocument()
    await screen.findByTestId('machine-checklist')
    await user.click(screen.getByRole('button', { name: 'Skip for now' }))
    expect(onNext).toHaveBeenCalledTimes(1)
    await user.click(screen.getByRole('button', { name: 'Back' }))
    expect(onBack).toHaveBeenCalledTimes(1)
  })

  it('the onboarding step continues once Docker answers', async () => {
    vi.mocked(commands.checkMachine).mockResolvedValue({
      status: 'ok',
      data: ready,
    })
    render(<StepComputer onNext={vi.fn()} onBack={vi.fn()} />)
    expect(
      await screen.findByRole('button', { name: 'Continue' })
    ).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Skip for now' })).toBeNull()
  })

  it('Home shows a calm Set up line until Docker is ready', async () => {
    const user = userEvent.setup()
    render(<MachineHomeItem />)
    const item = await screen.findByTestId('home-machine')
    expect(item).toHaveTextContent('Agents can talk, but need Docker to act.')
    await user.click(within(item).getByRole('button', { name: 'Set up' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'computer',
    })
  })

  it('Home shows nothing once Docker is ready', async () => {
    vi.mocked(commands.checkMachine).mockResolvedValue({
      status: 'ok',
      data: ready,
    })
    const { container } = render(<MachineHomeItem />)
    await vi.waitFor(() => expect(commands.checkMachine).toHaveBeenCalled())
    expect(container).toBeEmptyDOMElement()
  })
})

describe('the agent warning', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('"Tool execution unavailable" becomes "Agents need Docker to act. Set up"', async () => {
    const user = userEvent.setup()
    const setAgentWarning = vi.spyOn(
      useAgentStore.getState(),
      'setAgentWarning'
    )
    render(
      <AgentWarningBanner
        agentId="writer"
        warning="Tool execution unavailable: Docker is not running"
      />
    )
    const prompt = screen.getByTestId('docker-setup-prompt')
    expect(prompt).toHaveTextContent('Agents need Docker to act.')
    expect(prompt).not.toHaveTextContent('Tool execution unavailable')
    await user.click(within(prompt).getByRole('button', { name: 'Set up' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'computer',
    })
    await user.click(within(prompt).getByRole('button', { name: 'Dismiss' }))
    expect(setAgentWarning).toHaveBeenCalledWith('writer', null)
  })

  it('other warnings show as they are', () => {
    render(
      <AgentWarningBanner
        agentId="writer"
        warning="Connector Outlook left out"
      />
    )
    expect(screen.queryByTestId('docker-setup-prompt')).toBeNull()
    expect(screen.getByText('Connector Outlook left out')).toBeInTheDocument()
  })
})
