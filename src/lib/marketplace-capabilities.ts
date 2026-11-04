import type { ManifestCapabilities } from '@/lib/bindings'

/** The tools the agent runtime provides */
export const RUNTIME_TOOLS = [
  'shell',
  'read_file',
  'write_file',
  'list_dir',
] as const

type RuntimeTool = (typeof RUNTIME_TOOLS)[number]

const isRuntimeTool = (tool: string): tool is RuntimeTool =>
  (RUNTIME_TOOLS as readonly string[]).includes(tool)

const CAN = {
  shell: 'marketplace.can.shell',
  read_file: 'marketplace.can.readFile',
  write_file: 'marketplace.can.writeFile',
  list_dir: 'marketplace.can.listDir',
} as const

const CANNOT = {
  shell: 'marketplace.cannot.shell',
  read_file: 'marketplace.cannot.readFile',
  write_file: 'marketplace.cannot.writeFile',
  list_dir: 'marketplace.cannot.listDir',
} as const

export type NoticeKey =
  | (typeof CAN)[RuntimeTool]
  | (typeof CANNOT)[RuntimeTool]
  | 'marketplace.can.otherTool'
  | 'marketplace.can.network'
  | 'marketplace.can.aws'
  | 'marketplace.can.connectors'
  | 'marketplace.can.approval'
  | 'marketplace.cannot.network'
  | 'marketplace.cannot.otherHosts'
  | 'marketplace.cannot.aws'
  | 'marketplace.cannot.outsideWorkdir'

export interface Notice {
  key: NoticeKey
  values?: Record<string, string>
}

/**
 * The first-use notice: what an agent will be able to do, and what it
 * won't, from what it declares. Anything not declared is denied by the
 * runtime and the egress proxy, so the "cannot" list is a real guarantee.
 */
export function capabilityNotice(capabilities: ManifestCapabilities): {
  can: Notice[]
  cannot: Notice[]
} {
  const { tools, network, aws, connectors, requiresApproval } = capabilities
  const can: Notice[] = tools.map(tool =>
    isRuntimeTool(tool)
      ? { key: CAN[tool] }
      : { key: 'marketplace.can.otherTool', values: { tool } }
  )
  if (network.length > 0) {
    can.push({
      key: 'marketplace.can.network',
      values: { hosts: network.join(', ') },
    })
  }
  if (aws.length > 0) {
    can.push({
      key: 'marketplace.can.aws',
      values: { services: aws.join(', ') },
    })
  }
  if (connectors.length > 0) {
    can.push({
      key: 'marketplace.can.connectors',
      values: { connectors: connectors.join(', ') },
    })
  }
  const asks = requiresApproval.filter(tool => tools.includes(tool))
  if (asks.length > 0) {
    can.push({
      key: 'marketplace.can.approval',
      values: { tools: asks.join(', ') },
    })
  }

  const cannot: Notice[] = RUNTIME_TOOLS.filter(
    tool => !tools.includes(tool)
  ).map(tool => ({ key: CANNOT[tool] }))
  cannot.push({
    key:
      network.length === 0
        ? 'marketplace.cannot.network'
        : 'marketplace.cannot.otherHosts',
  })
  if (aws.length === 0) cannot.push({ key: 'marketplace.cannot.aws' })
  cannot.push({ key: 'marketplace.cannot.outsideWorkdir' })
  return { can, cannot }
}
