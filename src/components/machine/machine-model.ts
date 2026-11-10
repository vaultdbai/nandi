/**
 * "Get your computer ready": what the checklist says for each check the
 * core reports (`check_machine`). Pure. The core decides each check's
 * state and reason; this picks the plain sentence (why it matters) and the
 * numbered steps for this computer's OS (and Linux distribution).
 *
 * Phase 1 steps are instructions, copyable commands, official links and
 * Start Docker. Phase 2 adds approved "Install for me" actions to the
 * core's `MachineAction`; `MachineActions` renders them, and the steps
 * here stay as the manual way.
 */
import type {
  CheckId,
  CheckState,
  MachineAction,
  MachineCheck,
  MachineReport,
} from '@/lib/bindings'
import type en from '../../../locales/en.json'

/** The checklist re-checks this often while it's open */
export const MACHINE_POLL_MS = 10_000

type Keys = keyof typeof en
/** The steps' i18n keys, under `machine.step.` */
export type StepKey = {
  [K in Keys]: K extends `machine.step.${infer S}` ? S : never
}[Keys]

/** One numbered step: an i18n key (under `machine.step.`), maybe a command */
export interface MachineStep {
  key: StepKey
  params?: Record<string, string>
  /** Shown as a copyable command, left to right in every language */
  command?: string
}

/** The agent warning the core sends when a run's tools can't start */
const TOOLS_UNAVAILABLE = 'Tool execution unavailable'

/** Whether an `agent://warning` means "no tools: Docker isn't ready" */
export function isToolsUnavailable(warning: string | null): boolean {
  return warning?.startsWith(TOOLS_UNAVAILABLE) ?? false
}

export function notReady(report: MachineReport): MachineCheck[] {
  return report.checks.filter(check => check.state !== 'ok')
}

/** What the row's badge says */
export const STATE_TONE: Record<CheckState, 'ok' | 'warning' | 'muted'> = {
  ok: 'ok',
  missing: 'warning',
  stopped: 'warning',
  needsUpdate: 'warning',
  blocked: 'warning',
  unknown: 'muted',
}

/** The i18n key of the one sentence saying why a check matters */
export function whyKey(id: CheckId): `machine.why.${CheckId}` {
  return `machine.why.${id}`
}

/** Download pages get "Download", everything else "Read how" */
type LinkKey = 'machine.link.download' | 'machine.link.docs'
type ActionKey =
  LinkKey | 'machine.action.startDocker' | 'machine.action.checkAgain'

export function linkLabelKey(url: string): LinkKey {
  return /\.dmg$|\/install\/|wsl\/install/.test(url)
    ? 'machine.link.download'
    : 'machine.link.docs'
}

export function actionKey(action: MachineAction): ActionKey {
  switch (action.kind) {
    case 'startDocker':
      return 'machine.action.startDocker'
    case 'checkAgain':
      return 'machine.action.checkAgain'
    case 'openLink':
      return linkLabelKey(action.url)
  }
}

const step = (
  key: StepKey,
  extra: Omit<MachineStep, 'key'> = {}
): MachineStep => ({ key, ...extra })

/** Docker Engine from the distribution's own packages, when there's one */
function linuxPackageCommand(report: MachineReport): string | null {
  const distro = report.distro
  switch (distro?.family) {
    case 'debian':
      return 'sudo apt-get update && sudo apt-get install -y docker.io'
    case 'arch':
      return 'sudo pacman -S --needed docker'
    case 'fedora':
      return distro.id === 'fedora' ? 'sudo dnf install -y moby-engine' : null
    default:
      return null
  }
}

const DOCKER_GROUP = 'sudo usermod -aG docker $USER'
const START_ENGINE = 'sudo systemctl start docker'

function installSteps(report: MachineReport): MachineStep[] {
  switch (report.os) {
    case 'windows':
      return [
        step('winDownload'),
        step('winInstaller'),
        step('winRestart'),
        step('checkAgain'),
      ]
    case 'macos':
      return [
        step(report.appleSilicon ? 'macDownloadArm' : 'macDownloadIntel'),
        step('macDrag'),
        step('macOpen'),
        step('checkAgain'),
      ]
    default: {
      const name = report.distro?.name ?? report.distro?.id ?? 'Linux'
      const command = linuxPackageCommand(report)
      return [
        step('linuxPage', { params: { distro: name } }),
        ...(command ? [step('linuxPackage', { command })] : []),
        step('linuxEnable', { command: 'sudo systemctl enable --now docker' }),
        step('linuxGroup', { command: DOCKER_GROUP }),
        step('logOutIn'),
      ]
    }
  }
}

function runningSteps(
  check: MachineCheck,
  report: MachineReport
): MachineStep[] {
  const desktop = check.actions.some(a => a.kind === 'startDocker')
  switch (check.reason) {
    case 'needsDocker':
      return [step('installFirst')]
    case 'starting':
      return [step('waitStarting')]
    case 'socketPermission':
      return [step('linuxGroup', { command: DOCKER_GROUP }), step('logOutIn')]
    case 'desktopFailed':
      return report.os === 'windows'
        ? [
            step('failedQuit'),
            step('failedWslUpdate', { command: 'wsl --update' }),
            step('failedWslShutdown', { command: 'wsl --shutdown' }),
            step('failedStart'),
            step('failedVirtualization'),
            step('failedReset'),
          ]
        : [step('failedQuit'), step('failedStart'), step('failedReset')]
    default:
      if (report.os === 'linux' && !desktop)
        return [
          step('linuxStart', { command: START_ENGINE }),
          step('linuxStartAtBoot', { command: 'sudo systemctl enable docker' }),
        ]
      return [step('startDocker'), step('waitRunning')]
  }
}

/** The numbered steps for one check that isn't ready, on this computer */
export function stepsFor(
  check: MachineCheck,
  report: MachineReport
): MachineStep[] {
  if (check.state === 'ok') return []
  switch (check.id) {
    case 'dockerInstalled':
      if (check.reason === 'podman') return [step('podman')]
      if (check.reason === 'rootless') return [step('rootless')]
      if (check.reason === 'wrongBuild')
        return [step('wrongBuild'), step('macDrag'), step('macOpen')]
      return installSteps(report)
    case 'dockerRunning':
      return runningSteps(check, report)
    case 'wsl':
      if (check.reason === 'notInstalled')
        return [
          step('wslInstall', { command: 'wsl --install --no-distribution' }),
          step('restartWhenReady'),
        ]
      if (check.reason === 'notChecked')
        return [step('wslStatus', { command: 'wsl --status' })]
      return [
        step('wslUpdate', { command: 'wsl --update' }),
        step('failedWslShutdown', { command: 'wsl --shutdown' }),
      ]
    case 'virtualization':
      if (check.reason === 'notChecked') return [step('virtCheck')]
      if (report.os === 'macos') return [step('virtMacIntel')]
      return [step('virtFirmware'), step('virtTurnOn'), step('virtRestart')]
    case 'disk':
      return check.reason === 'lowDisk' ? [step('diskFree')] : []
    case 'images':
      switch (check.reason) {
        case 'unreachable':
          return [step('imagesOnline'), step('imagesFirewall')]
        case 'certificate':
          return [step('imagesCertificate')]
        case 'proxyAuth':
          return [step('imagesProxyAuth')]
        case 'refused':
          return [step('imagesRefused')]
        default:
          return []
      }
  }
}

/** Actions shown as buttons: Check again lives in the page header */
export function rowActions(check: MachineCheck): MachineAction[] {
  return check.actions.filter(action => action.kind !== 'checkAgain')
}
