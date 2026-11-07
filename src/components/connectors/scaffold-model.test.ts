import { describe, expect, it } from 'vitest'
import type { RegistryInfo } from '@/lib/bindings'
import {
  checkScaffold,
  EMPTY_SCAFFOLD,
  publishTargets,
  runProblem,
  scaffoldInput,
  validHost,
  validId,
  type ScaffoldForm,
} from './scaffold-model'

const FORM: ScaffoldForm = {
  ...EMPTY_SCAFFOLD,
  id: 'tracker',
  name: 'Issue tracker',
  description: 'Read and file issues.',
  run: 'npx -y @acme/tracker-mcp@1.2.3',
  auth: 'api_key',
  authEnv: 'TRACKER_TOKEN',
  authLabel: 'Tracker API key',
  apiKey: ' secret ',
  hosts: 'api.tracker.example\n',
  tools: [
    { name: 'list_issues', description: 'List open issues', access: 'read' },
  ],
  reads: 'Issues in your projects',
  tags: 'issues, work',
}

const fields = (form: ScaffoldForm) => checkScaffold(form).map(e => e.field)

describe('runProblem', () => {
  it('takes exact npx and uvx versions', () => {
    expect(runProblem('npx -y @acme/tracker-mcp@1.2.3')).toBeNull()
    expect(runProblem('npx pkg@1.0.0 --flag')).toBeNull()
    expect(runProblem('uvx mcp-server-time==1.0')).toBeNull()
  })

  it('refuses ranges, latest and other runners', () => {
    expect(runProblem('npx -y @acme/tracker-mcp@latest')).toBe('runVersion')
    expect(runProblem('npx pkg@1.2')).toBe('runVersion')
    expect(runProblem('npx pkg@1.x.0')).toBe('runVersion')
    expect(runProblem('uvx pkg')).toBe('runVersion')
    expect(runProblem('node server.js')).toBe('runStart')
    expect(runProblem('npx -y')).toBe('runPackage')
    expect(runProblem('npx @scope@1.0.0')).toBe('runPackage')
  })
})

describe('validHost and validId', () => {
  it('follows the Rust rules', () => {
    expect(validHost('api.tracker.example')).toBe(true)
    expect(validHost('*.tracker.example:443')).toBe(true)
    expect(validHost('localhost')).toBe(false)
    expect(validHost('API.example')).toBe(false)
    expect(validHost('a.example:')).toBe(false)
    expect(validId('tracker-v2')).toBe(true)
    expect(validId('.hidden')).toBe(false)
    expect(validId('has space')).toBe(false)
  })
})

describe('checkScaffold', () => {
  it('passes a complete form', () => {
    expect(checkScaffold(FORM)).toEqual([])
  })

  it('needs an id, name, description and x.y.z version', () => {
    expect(
      fields({ ...FORM, id: '', name: ' ', description: '', version: '1.0' })
    ).toEqual(['id', 'name', 'description', 'version'])
  })

  it('needs an https server with no image', () => {
    expect(
      fields({ ...FORM, server: 'url', url: 'http://x.example', image: 'i' })
    ).toEqual(['url', 'image'])
    expect(
      fields({ ...FORM, server: 'url', url: 'https://mcp.example/sse' })
    ).toEqual([])
  })

  it('checks hosts, env names and tools', () => {
    const errors = checkScaffold({
      ...FORM,
      hosts: 'ok.example\nBad Host',
      authEnv: 'lower',
      tools: [
        { name: 'a', description: 'x', access: 'read' },
        { name: 'a', description: 'y', access: 'write' },
        { name: 'b', description: '', access: 'write' },
      ],
    })
    expect(errors.map(e => e.key)).toEqual([
      'host',
      'env',
      'toolTwice',
      'toolDescription',
    ])
    expect(errors[0]?.values).toEqual({ host: 'Bad Host' })
  })

  it('needs the variable an API key goes into', () => {
    expect(checkScaffold({ ...FORM, authEnv: '' }).map(e => e.key)).toEqual([
      'envRequired',
    ])
  })

  it('allows no tools: Nandi lists them', () => {
    expect(checkScaffold({ ...FORM, tools: [] })).toEqual([])
  })

  it('keeps instructions to 600 words', () => {
    expect(fields({ ...FORM, instructions: 'word '.repeat(601) })).toEqual([
      'instructions',
    ])
  })
})

describe('scaffoldInput', () => {
  it('trims and splits the form', () => {
    expect(scaffoldInput(FORM)).toEqual({
      id: 'tracker',
      name: 'Issue tracker',
      description: 'Read and file issues.',
      version: '0.1.0',
      run: 'npx -y @acme/tracker-mcp@1.2.3',
      url: null,
      image: null,
      auth: {
        kind: 'api_key',
        env: ['TRACKER_TOKEN'],
        scopes: [],
        label: 'Tracker API key',
      },
      hosts: ['api.tracker.example'],
      tools: [
        {
          name: 'list_issues',
          description: 'List open issues',
          access: 'read',
        },
      ],
      reads: ['Issues in your projects'],
      writes: [],
      tags: ['issues', 'work'],
      instructions: '',
      apiKey: 'secret',
    })
  })

  it('sends a URL server and OAuth scopes', () => {
    const input = scaffoldInput({
      ...FORM,
      server: 'url',
      url: 'https://mcp.example/sse',
      auth: 'oauth',
      authScopes: 'read:issues write:issues',
    })
    expect(input.run).toBeNull()
    expect(input.url).toBe('https://mcp.example/sse')
    expect(input.auth?.scopes).toEqual(['read:issues', 'write:issues'])
    expect(input.apiKey).toBeNull()
  })
})

describe('publishTargets', () => {
  it('puts the public registry last', () => {
    const registry = (url: string, kind: string) =>
      ({ url, kind }) as RegistryInfo
    expect(
      publishTargets([
        registry('pub', 'public'),
        registry('mine', 'community'),
      ]).map(r => r.url)
    ).toEqual(['mine', 'pub'])
  })
})
