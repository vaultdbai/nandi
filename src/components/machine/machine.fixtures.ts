/** Test data for the machine checklist */
import type { MachineCheck, MachineReport } from '@/lib/bindings'

export const check = (
  over: Partial<MachineCheck> & Pick<MachineCheck, 'id'>
): MachineCheck => ({
  state: 'ok',
  reason: 'ready',
  detail: '',
  actions: [],
  ...over,
})

export const report = (over: Partial<MachineReport> = {}): MachineReport => ({
  os: 'windows',
  arch: 'x86_64',
  appleSilicon: false,
  distro: null,
  checks: [],
  dockerReady: true,
  ready: true,
  checkedAt: '2026-11-09T12:00:00Z',
  ...over,
})
