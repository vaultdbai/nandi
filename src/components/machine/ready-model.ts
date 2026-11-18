/**
 * "Get your computer ready" with one go-ahead (phase 2, Windows and
 * macOS). Pure. The core's report carries the `plan` (the steps, in
 * order); this decides when the card shows, what it says, and how a run
 * is going (`computer_ready_status`, `machine://ready`).
 */
import type {
  MachineOs,
  MachineReport,
  MachineSettings,
  ReadyProgress,
  ReadyStep,
} from '@/lib/bindings'

/** Steps that install or update something, and so need the go-ahead */
const INSTALLS: ReadyStep[] = ['wslUpdate', 'wslInstall', 'installDocker']

/** The steps to approve, or none (a lone Start Docker is the button) */
export function goAheadPlan(report: MachineReport | undefined): ReadyStep[] {
  const plan = report?.plan ?? []
  return plan.some(step => INSTALLS.includes(step)) ? plan : []
}

type TitleKey =
  | 'machine.ready.title.updateWslInstallDocker'
  | 'machine.ready.title.installWslInstallDocker'
  | 'machine.ready.title.installDocker'
  | 'machine.ready.title.updateWsl'
  | 'machine.ready.title.installWsl'

/** "Get your computer ready? Nandi will update WSL and install Docker." */
export function titleKey(plan: ReadyStep[]): TitleKey {
  const docker = plan.includes('installDocker')
  if (plan.includes('wslInstall'))
    return docker
      ? 'machine.ready.title.installWslInstallDocker'
      : 'machine.ready.title.installWsl'
  if (plan.includes('wslUpdate'))
    return docker
      ? 'machine.ready.title.updateWslInstallDocker'
      : 'machine.ready.title.updateWsl'
  return 'machine.ready.title.installDocker'
}

export function stepKey(step: ReadyStep): `machine.ready.step.${ReadyStep}` {
  return `machine.ready.step.${step}`
}

/** Who asks for admin rights: the OS, once per installer */
export function adminKey(
  os: MachineOs
): 'machine.ready.adminWindows' | 'machine.ready.adminMac' {
  return os === 'macos'
    ? 'machine.ready.adminMac'
    : 'machine.ready.adminWindows'
}

/** The Docker licence line goes with a plan that installs Docker */
export function showsLicence(plan: ReadyStep[]): boolean {
  return plan.includes('installDocker')
}

/** What the card shows */
export type ReadyView =
  | { kind: 'ask'; plan: ReadyStep[] }
  | { kind: 'running'; progress: ReadyProgress }
  | { kind: 'failed'; progress: ReadyProgress }
  | { kind: 'handedOver'; progress: ReadyProgress }
  | { kind: 'done'; progress: ReadyProgress }
  | { kind: 'hidden' }

/**
 * The card's state: a run in progress or just ended wins over a new
 * question; "Not now" (`dismissed`) hides the question and the "done"
 * line, never a run or what stopped it.
 */
export function readyView(
  report: MachineReport | undefined,
  progress: ReadyProgress | undefined,
  dismissed: boolean
): ReadyView {
  if (progress?.running) return { kind: 'running', progress }
  if (progress?.failure) return { kind: 'failed', progress }
  if (progress?.handover) return { kind: 'handedOver', progress }
  const plan = goAheadPlan(report)
  if (plan.length > 0 && !dismissed) return { kind: 'ask', plan }
  if (progress && progress.steps.length > 0 && plan.length === 0 && !dismissed)
    return { kind: 'done', progress }
  return { kind: 'hidden' }
}

type HandoverKey = `machine.ready.handover.${NonNullable<
  ReadyProgress['handover']
>}`

export function handoverKey(
  handover: NonNullable<ReadyProgress['handover']>
): HandoverKey {
  return `machine.ready.handover.${handover}`
}

/**
 * Ask "Start Docker automatically when Nandi opens?" the first time the
 * check finds Docker stopped (not starting, not failed), until answered.
 */
export function asksStartOnOpen(
  report: MachineReport | undefined,
  settings: MachineSettings | undefined
): boolean {
  if (!report || !settings || settings.startDockerOnOpen !== null) return false
  return report.checks.some(
    check => check.id === 'dockerRunning' && check.reason === 'notRunning'
  )
}
