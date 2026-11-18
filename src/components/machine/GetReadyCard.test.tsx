import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, within } from '@/test/test-utils'
import { emitMockEvent } from '@/test/mock-events'
import { commands } from '@/lib/tauri-bindings'
import type { ReadyProgress, ReadyStep } from '@/lib/bindings'
import { MachineChecklist } from './MachineChecklist'
import { check, report } from './machine.fixtures'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const plan: ReadyStep[] = ['wslUpdate', 'installDocker', 'startDocker']

/** Windows with no Docker: the one go-ahead */
const noDocker = report({
  dockerReady: false,
  ready: false,
  plan,
  checks: [
    check({ id: 'dockerInstalled', state: 'missing', reason: 'notInstalled' }),
    check({ id: 'dockerRunning', state: 'missing', reason: 'needsDocker' }),
    check({ id: 'wsl', detail: 'WSL 3.0.5.0' }),
  ],
})

const stopped = report({
  dockerReady: false,
  ready: false,
  checks: [
    check({ id: 'dockerInstalled' }),
    check({
      id: 'dockerRunning',
      state: 'stopped',
      reason: 'notRunning',
      actions: [{ kind: 'startDocker' }, { kind: 'checkAgain' }],
    }),
  ],
})

const idle: ReadyProgress = {
  running: false,
  steps: [],
  log: [],
  handover: null,
  handoverLink: null,
  failure: null,
  report: null,
}

describe('GetReadyCard', () => {
  beforeEach(() => {
    Object.assign(commands, {
      computerReadyStatus: ok(idle),
      getComputerReady: ok(null),
      machineSettings: ok({ startDockerOnOpen: false }),
      setStartDockerOnOpen: ok(null),
      startDocker: ok(null),
    })
  })

  it('asks once, with the steps, admin rights and Docker’s licence', async () => {
    const user = userEvent.setup()
    render(<MachineChecklist report={noDocker} />)
    const card = await screen.findByTestId('get-ready-ask')
    expect(card).toHaveTextContent(
      'Get your computer ready? Nandi will update WSL and install Docker.'
    )
    const steps = within(card).getAllByRole('listitem')
    expect(steps.map(s => s.textContent)).toEqual([
      'Update WSL (wsl --update)',
      'Install Docker Desktop, downloaded from Docker',
      'Start Docker',
    ])
    expect(card).toHaveTextContent('Windows asks for admin rights')
    expect(card).toHaveTextContent(
      'Companies with more than 250 employees or $10M in revenue need a Docker subscription'
    )
    await user.click(within(card).getByRole('button', { name: 'Go ahead' }))
    expect(commands.getComputerReady).toHaveBeenCalledWith(plan)
  })

  it('Not now keeps the checklist', async () => {
    const user = userEvent.setup()
    render(<MachineChecklist report={noDocker} />)
    const card = await screen.findByTestId('get-ready-ask')
    await user.click(within(card).getByRole('button', { name: 'Not now' }))
    expect(screen.queryByTestId('get-ready-ask')).toBeNull()
    expect(screen.getByTestId('machine-checklist')).toBeInTheDocument()
    expect(commands.getComputerReady).not.toHaveBeenCalled()
  })

  it('shows the run live, and a failed step with what Nandi saw', async () => {
    render(<MachineChecklist report={noDocker} />)
    await screen.findByTestId('get-ready-ask')
    act(() =>
      emitMockEvent('machine://ready', {
        ...idle,
        running: true,
        steps: [
          { step: 'wslUpdate', status: 'done' },
          { step: 'installDocker', status: 'running' },
          { step: 'startDocker', status: 'waiting' },
        ],
        log: ['Found Docker Desktop [Docker.DockerDesktop]'],
      })
    )
    const running = await screen.findByTestId('get-ready-running')
    expect(running).toHaveTextContent('Found Docker Desktop')
    act(() =>
      emitMockEvent('machine://ready', {
        ...idle,
        steps: [
          { step: 'wslUpdate', status: 'done' },
          { step: 'installDocker', status: 'failed' },
          { step: 'startDocker', status: 'waiting' },
        ],
        failure: {
          step: 'installDocker',
          detail:
            'winget exited with 1. Nandi checked again and saw: No Docker found',
          link: 'https://docs.docker.com/desktop/setup/install/windows-install/',
        },
      })
    )
    const failed = await screen.findByTestId('get-ready-failed')
    expect(failed).toHaveTextContent('No Docker found')
    expect(
      within(failed).getByRole('button', { name: 'Do it yourself' })
    ).toBeInTheDocument()
    expect(
      within(failed).getByRole('button', { name: 'Try again' })
    ).toBeInTheDocument()
  })

  it('asks to start Docker when Nandi opens, once', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.machineSettings).mockResolvedValue({
      status: 'ok',
      data: { startDockerOnOpen: null },
    })
    render(<MachineChecklist report={stopped} />)
    const question = await screen.findByTestId('start-on-open-question')
    expect(question).toHaveTextContent(
      'Start Docker automatically when Nandi opens?'
    )
    await user.click(
      within(question).getByRole('button', { name: 'Yes, start it' })
    )
    expect(commands.setStartDockerOnOpen).toHaveBeenCalledWith(true)
    expect(commands.startDocker).toHaveBeenCalled()
  })

  it('doesn’t ask again once answered', async () => {
    render(<MachineChecklist report={stopped} />)
    await screen.findByTestId('machine-checklist')
    await vi.waitFor(() => expect(commands.machineSettings).toHaveBeenCalled())
    expect(screen.queryByTestId('start-on-open-question')).toBeNull()
  })
})
