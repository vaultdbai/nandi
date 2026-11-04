import { describe, expect, it } from 'vitest'
import { capabilityNotice } from './marketplace-capabilities'
import { compareVersions } from './marketplace-format'

const caps = {
  tools: ['shell', 'read_file'],
  network: ['arxiv.org'],
  pilots: [],
  requiresApproval: ['shell', 'write_file'],
  aws: [],
  connectors: [],
}

describe('capabilityNotice', () => {
  it('lists what is allowed, what asks first, and what is denied', () => {
    const { can, cannot } = capabilityNotice(caps)
    expect(can.map(n => n.key)).toEqual([
      'marketplace.can.shell',
      'marketplace.can.readFile',
      'marketplace.can.network',
      'marketplace.can.approval',
    ])
    // write_file isn't allowed at all, so it isn't "asks first"
    expect(can.at(-1)?.values).toEqual({ tools: 'shell' })
    expect(cannot.map(n => n.key)).toEqual([
      'marketplace.cannot.writeFile',
      'marketplace.cannot.listDir',
      'marketplace.cannot.otherHosts',
      'marketplace.cannot.aws',
      'marketplace.cannot.outsideWorkdir',
    ])
  })

  it('says no internet when no hosts are listed', () => {
    const { cannot } = capabilityNotice({ ...caps, network: [] })
    expect(cannot.map(n => n.key)).toContain('marketplace.cannot.network')
  })

  it('names the connectors a pack uses', () => {
    const { can } = capabilityNotice({ ...caps, connectors: ['s3_backup'] })
    expect(can).toContainEqual({
      key: 'marketplace.can.connectors',
      values: { connectors: 's3_backup' },
    })
  })
})

describe('compareVersions', () => {
  it('compares numerically, ignoring a v prefix', () => {
    expect(compareVersions('1.10.0', '1.9.0')).toBeGreaterThan(0)
    expect(compareVersions('v1.2.0', '1.2.0')).toBe(0)
    expect(compareVersions('1.2.0', '2.0.0')).toBeLessThan(0)
  })
})

describe('nextVersion', () => {
  it('bumps the patch, or starts at 1.0.0', async () => {
    const { nextVersion } = await import('./marketplace-format')
    expect(nextVersion('1.2.3')).toBe('1.2.4')
    expect(nextVersion('v2.0.9')).toBe('2.0.10')
    expect(nextVersion(undefined)).toBe('1.0.0')
  })
})
