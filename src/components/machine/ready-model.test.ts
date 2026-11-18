import { describe, expect, it } from 'vitest'
import type { ReadyProgress, ReadyStep } from '@/lib/bindings'
import { check, report } from './machine.fixtures'
import {
  adminKey,
  asksStartOnOpen,
  goAheadPlan,
  handoverKey,
  readyView,
  showsLicence,
  stepKey,
  titleKey,
} from './ready-model'

const progress = (over: Partial<ReadyProgress> = {}): ReadyProgress => ({
  running: false,
  steps: [],
  log: [],
  handover: null,
  handoverLink: null,
  failure: null,
  report: null,
  ...over,
})

const docker: ReadyStep[] = ['wslUpdate', 'installDocker', 'startDocker']

describe('ready model', () => {
  it('asks for a go-ahead only when the plan installs or updates', () => {
    expect(goAheadPlan(report({ plan: docker }))).toEqual(docker)
    expect(goAheadPlan(report({ plan: ['wslInstall', 'restart'] }))).toEqual([
      'wslInstall',
      'restart',
    ])
    // A lone Start Docker is the checklist's button
    expect(goAheadPlan(report({ plan: ['startDocker'] }))).toEqual([])
    expect(goAheadPlan(report())).toEqual([])
    expect(goAheadPlan(undefined)).toEqual([])
  })

  it('says what Nandi will do in the title', () => {
    expect(titleKey(docker)).toBe('machine.ready.title.updateWslInstallDocker')
    expect(titleKey(['wslInstall', 'installDocker', 'restart'])).toBe(
      'machine.ready.title.installWslInstallDocker'
    )
    expect(titleKey(['installDocker', 'startDocker'])).toBe(
      'machine.ready.title.installDocker'
    )
    expect(titleKey(['wslUpdate', 'startDocker'])).toBe(
      'machine.ready.title.updateWsl'
    )
    expect(titleKey(['wslInstall', 'restart'])).toBe(
      'machine.ready.title.installWsl'
    )
    expect(stepKey('wslUpdate')).toBe('machine.ready.step.wslUpdate')
    expect(handoverKey('dmg')).toBe('machine.ready.handover.dmg')
  })

  it('names who asks for admin rights, and the licence goes with Docker', () => {
    expect(adminKey('windows')).toBe('machine.ready.adminWindows')
    expect(adminKey('macos')).toBe('machine.ready.adminMac')
    expect(showsLicence(docker)).toBe(true)
    expect(showsLicence(['wslUpdate'])).toBe(false)
  })

  it('a run wins over the question, and Not now hides only the question', () => {
    const r = report({ plan: docker })
    expect(readyView(r, undefined, false)).toEqual({
      kind: 'ask',
      plan: docker,
    })
    expect(readyView(r, progress(), true).kind).toBe('hidden')

    const steps = docker.map(step => ({ step, status: 'waiting' as const }))
    const running = progress({ running: true, steps })
    expect(readyView(r, running, true).kind).toBe('running')

    const failed = progress({
      steps,
      failure: { step: 'installDocker', detail: 'winget exited', link: 'x' },
    })
    expect(readyView(r, failed, true).kind).toBe('failed')
    expect(
      readyView(r, progress({ steps, handover: 'restart' }), true).kind
    ).toBe('handedOver')

    // Finished, and nothing left to do
    const done = progress({ steps })
    expect(readyView(report(), done, false).kind).toBe('done')
    expect(readyView(report(), done, true).kind).toBe('hidden')
    // Finished, and the check still finds something: ask again
    expect(readyView(r, done, false).kind).toBe('ask')
  })

  it('asks to start Docker on open once, when Docker is stopped', () => {
    const stopped = report({
      checks: [
        check({ id: 'dockerRunning', state: 'stopped', reason: 'notRunning' }),
      ],
    })
    expect(asksStartOnOpen(stopped, { startDockerOnOpen: null })).toBe(true)
    expect(asksStartOnOpen(stopped, { startDockerOnOpen: false })).toBe(false)
    expect(asksStartOnOpen(stopped, { startDockerOnOpen: true })).toBe(false)
    expect(asksStartOnOpen(stopped, undefined)).toBe(false)
    const starting = report({
      checks: [
        check({ id: 'dockerRunning', state: 'stopped', reason: 'starting' }),
      ],
    })
    expect(asksStartOnOpen(starting, { startDockerOnOpen: null })).toBe(false)
  })
})
