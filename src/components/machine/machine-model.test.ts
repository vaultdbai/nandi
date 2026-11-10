import { describe, expect, it } from 'vitest'
import type { MachineCheck, MachineReport } from '@/lib/bindings'
import { check, report } from './machine.fixtures'
import {
  actionKey,
  isToolsUnavailable,
  linkLabelKey,
  notReady,
  rowActions,
  stepsFor,
} from './machine-model'

const keys = (c: MachineCheck, r: MachineReport) =>
  stepsFor(c, r).map(s => s.key)

describe('machine model', () => {
  it('recognises the core’s tools-unavailable warning only', () => {
    expect(
      isToolsUnavailable('Tool execution unavailable: Docker is not running')
    ).toBe(true)
    expect(isToolsUnavailable('Connector Outlook left out')).toBe(false)
    expect(isToolsUnavailable(null)).toBe(false)
  })

  it('lists what isn’t ready', () => {
    const r = report({
      checks: [
        check({ id: 'dockerInstalled' }),
        check({ id: 'disk', state: 'blocked', reason: 'lowDisk' }),
      ],
    })
    expect(notReady(r).map(c => c.id)).toEqual(['disk'])
    expect(stepsFor(check({ id: 'dockerInstalled' }), r)).toEqual([])
  })

  it('gives install steps per OS, the chip on a Mac', () => {
    const missing = check({
      id: 'dockerInstalled',
      state: 'missing',
      reason: 'notInstalled',
    })
    expect(keys(missing, report())).toEqual([
      'winDownload',
      'winInstaller',
      'winRestart',
      'checkAgain',
    ])
    expect(keys(missing, report({ os: 'macos', appleSilicon: true }))[0]).toBe(
      'macDownloadArm'
    )
    expect(keys(missing, report({ os: 'macos' }))[0]).toBe('macDownloadIntel')
  })

  it('tailors Linux steps to the distribution family', () => {
    const missing = check({
      id: 'dockerInstalled',
      state: 'missing',
      reason: 'notInstalled',
    })
    const linux = (
      id: string,
      family: 'debian' | 'fedora' | 'arch' | 'other'
    ) =>
      report({
        os: 'linux',
        distro: { id, idLike: [], versionId: null, name: null, family },
      })
    const ubuntu = stepsFor(missing, linux('ubuntu', 'debian'))
    expect(ubuntu[0]).toEqual({
      key: 'linuxPage',
      params: { distro: 'ubuntu' },
    })
    expect(ubuntu.find(s => s.key === 'linuxPackage')?.command).toBe(
      'sudo apt-get update && sudo apt-get install -y docker.io'
    )
    expect(ubuntu.find(s => s.key === 'linuxGroup')?.command).toBe(
      'sudo usermod -aG docker $USER'
    )
    expect(
      stepsFor(missing, linux('arch', 'arch')).find(
        s => s.key === 'linuxPackage'
      )?.command
    ).toBe('sudo pacman -S --needed docker')
    expect(
      stepsFor(missing, linux('fedora', 'fedora')).find(
        s => s.key === 'linuxPackage'
      )?.command
    ).toBe('sudo dnf install -y moby-engine')
    // RHEL and unknown distributions: Docker's page only
    expect(keys(missing, linux('rhel', 'fedora'))).not.toContain('linuxPackage')
    expect(keys(missing, linux('nixos', 'other'))).not.toContain('linuxPackage')
  })

  it('starts Docker per OS: a button on desktops, a command for Linux Engine', () => {
    const stopped = check({
      id: 'dockerRunning',
      state: 'stopped',
      reason: 'notRunning',
      actions: [{ kind: 'startDocker' }, { kind: 'checkAgain' }],
    })
    expect(keys(stopped, report())).toEqual(['startDocker', 'waitRunning'])
    const engine = check({
      id: 'dockerRunning',
      state: 'stopped',
      reason: 'notRunning',
      actions: [{ kind: 'checkAgain' }],
    })
    expect(stepsFor(engine, report({ os: 'linux' }))[0]).toEqual({
      key: 'linuxStart',
      command: 'sudo systemctl start docker',
    })
  })

  it('walks through "Docker Desktop is unable to start" in order', () => {
    const failed = check({
      id: 'dockerRunning',
      state: 'blocked',
      reason: 'desktopFailed',
    })
    const steps = stepsFor(failed, report())
    expect(steps.map(s => s.key)).toEqual([
      'failedQuit',
      'failedWslUpdate',
      'failedWslShutdown',
      'failedStart',
      'failedVirtualization',
      'failedReset',
    ])
    expect(steps[1]?.command).toBe('wsl --update')
    expect(keys(failed, report({ os: 'macos' }))).toEqual([
      'failedQuit',
      'failedStart',
      'failedReset',
    ])
  })

  it('fixes the socket permission with the docker group', () => {
    const denied = check({
      id: 'dockerRunning',
      state: 'blocked',
      reason: 'socketPermission',
    })
    expect(keys(denied, report({ os: 'linux' }))).toEqual([
      'linuxGroup',
      'logOutIn',
    ])
  })

  it('covers WSL, virtualization, disk and images', () => {
    const r = report()
    expect(
      stepsFor(
        check({ id: 'wsl', state: 'missing', reason: 'notInstalled' }),
        r
      )[0]?.command
    ).toBe('wsl --install --no-distribution')
    expect(
      keys(check({ id: 'wsl', state: 'needsUpdate', reason: 'oldWsl' }), r)
    ).toEqual(['wslUpdate', 'failedWslShutdown'])
    expect(
      keys(
        check({
          id: 'virtualization',
          state: 'blocked',
          reason: 'virtualizationOff',
        }),
        r
      )
    ).toEqual(['virtFirmware', 'virtTurnOn', 'virtRestart'])
    expect(
      keys(check({ id: 'images', state: 'blocked', reason: 'certificate' }), r)
    ).toEqual(['imagesCertificate'])
    expect(
      keys(check({ id: 'disk', state: 'blocked', reason: 'lowDisk' }), r)
    ).toEqual(['diskFree'])
  })

  it('labels links and leaves Check again to the page', () => {
    expect(
      linkLabelKey('https://docs.docker.com/desktop/setup/install/mac-install/')
    ).toBe('machine.link.download')
    expect(
      linkLabelKey('https://desktop.docker.com/mac/main/arm64/Docker.dmg')
    ).toBe('machine.link.download')
    expect(
      linkLabelKey(
        'https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/'
      )
    ).toBe('machine.link.docs')
    expect(actionKey({ kind: 'startDocker' })).toBe(
      'machine.action.startDocker'
    )
    expect(
      rowActions(
        check({
          id: 'dockerRunning',
          actions: [{ kind: 'startDocker' }, { kind: 'checkAgain' }],
        })
      )
    ).toEqual([{ kind: 'startDocker' }])
  })
})
