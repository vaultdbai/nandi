/**
 * Studio › New connector: the form, its checks and the scaffold input.
 * The checks mirror `domain/connector_package.rs` (exact versions, https
 * servers, host names, env names, tools) so mistakes show before saving;
 * Rust checks again. Pure.
 */

import type {
  AuthKind,
  PackageTool,
  RegistryInfo,
  ScaffoldConnectorInput,
} from '@/lib/bindings'

export const MAX_INSTRUCTION_WORDS = 600
const MAX_DESCRIPTION = 200

export interface ToolDraft {
  name: string
  description: string
  access: 'read' | 'write'
}

export interface ScaffoldForm {
  id: string
  name: string
  description: string
  version: string
  server: 'package' | 'url'
  run: string
  url: string
  image: string
  auth: Extract<AuthKind, 'none' | 'api_key' | 'oauth'>
  /** Environment variable names, separated by commas or spaces */
  authEnv: string
  authLabel: string
  /** OAuth scopes, separated by spaces or commas */
  authScopes: string
  apiKey: string
  /** One per line */
  hosts: string
  tools: ToolDraft[]
  /** One per line */
  reads: string
  writes: string
  /** Separated by commas */
  tags: string
  instructions: string
}

export const EMPTY_SCAFFOLD: ScaffoldForm = {
  id: '',
  name: '',
  description: '',
  version: '0.1.0',
  server: 'package',
  run: '',
  url: '',
  image: '',
  auth: 'none',
  authEnv: '',
  authLabel: '',
  authScopes: '',
  apiKey: '',
  hosts: '',
  tools: [],
  reads: '',
  writes: '',
  tags: '',
  instructions: '',
}

export type ScaffoldField =
  | 'id'
  | 'name'
  | 'description'
  | 'version'
  | 'run'
  | 'url'
  | 'image'
  | 'authEnv'
  | 'hosts'
  | 'tools'
  | 'instructions'

/** A key under `newConnector.error.` and its values */
export interface ScaffoldError {
  field: ScaffoldField
  key: string
  values?: Record<string, string | number>
}

export function lines(text: string): string[] {
  return text
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean)
}

export function words(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map(word => word.trim())
    .filter(Boolean)
}

/** `name_rule` in `domain/ids.rs` */
export function validId(id: string): boolean {
  return (
    id.length > 0 &&
    id.length <= 128 &&
    !id.startsWith('.') &&
    /^[A-Za-z0-9._-]+$/.test(id)
  )
}

/** x.y.z, with an optional pre-release or build */
export function semver(version: string): boolean {
  return /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(version)
}

function exactVersion(version: string, npm: boolean): boolean {
  const ok =
    /^\d[A-Za-z0-9.+-]*$/.test(version) &&
    !version.split('.').some(part => part === 'x' || part === 'X')
  return ok && (!npm || semver(version))
}

function validPackage(name: string): boolean {
  const part = (s: string, firstUpper: boolean) =>
    (firstUpper ? /^[a-zA-Z0-9]/ : /^[a-z0-9]/).test(s) &&
    /^[A-Za-z0-9._-]+$/.test(s)
  if (name.startsWith('@')) {
    const [scope, rest, ...more] = name.slice(1).split('/')
    return (
      more.length === 0 &&
      scope !== undefined &&
      rest !== undefined &&
      part(scope, false) &&
      part(rest, true)
    )
  }
  return part(name, true)
}

/**
 * `RunSpec::parse_command`: `npx [-y] <package>@<x.y.z>` or
 * `uvx <package>==<version>`. The error key, or null when it's fine
 */
export function runProblem(line: string): string | null {
  const [runner, ...rest] = line.trim().split(/\s+/)
  if (runner !== 'npx' && runner !== 'uvx') return 'runStart'
  let spec = rest[0]
  if (runner === 'npx' && (spec === '-y' || spec === '--yes')) spec = rest[1]
  if (!spec) return 'runPackage'
  let pkg: string
  let version: string
  if (runner === 'npx') {
    const at = spec.lastIndexOf('@')
    if (at <= 0) return 'runVersion'
    pkg = spec.slice(0, at)
    version = spec.slice(at + 1)
  } else {
    const split = spec.indexOf('==')
    if (split < 0) return 'runVersion'
    pkg = spec.slice(0, split)
    version = spec.slice(split + 2)
  }
  if (!validPackage(pkg)) return 'runPackage'
  return exactVersion(version, runner === 'npx') ? null : 'runVersion'
}

/** `valid_host`: lowercase labels, an optional `*.` and port */
export function validHost(host: string): boolean {
  const colon = host.lastIndexOf(':')
  let name = host
  if (colon >= 0) {
    const port = host.slice(colon + 1)
    if (!/^\d{1,5}$/.test(port)) return false
    name = host.slice(0, colon)
  }
  if (name.startsWith('*.')) name = name.slice(2)
  const labels = name.split('.')
  return (
    labels.length >= 2 &&
    labels.every(
      label =>
        label !== '' &&
        !label.startsWith('-') &&
        !label.endsWith('-') &&
        /^[a-z0-9-]+$/.test(label)
    )
  )
}

export function validEnvName(name: string): boolean {
  return /^[A-Z][A-Z0-9_]*$/.test(name)
}

function httpsUrl(text: string): boolean {
  if (!text.startsWith('https://')) return false
  try {
    return new URL(text).host !== ''
  } catch {
    return false
  }
}

export function checkScaffold(form: ScaffoldForm): ScaffoldError[] {
  const errors: ScaffoldError[] = []
  const add = (
    field: ScaffoldField,
    key: string,
    values?: ScaffoldError['values']
  ) => errors.push({ field, key, values })
  const id = form.id.trim()
  if (!id) add('id', 'required')
  else if (!validId(id)) add('id', 'id')
  if (!form.name.trim()) add('name', 'required')
  const description = form.description.trim()
  if (!description || description.length > MAX_DESCRIPTION) {
    add('description', 'description', { max: MAX_DESCRIPTION })
  }
  if (!semver(form.version.trim())) add('version', 'version')
  if (form.server === 'package') {
    const problem = form.run.trim() ? runProblem(form.run) : 'required'
    if (problem) add('run', problem)
  } else {
    if (!httpsUrl(form.url.trim())) add('url', 'url')
    if (form.image.trim()) add('image', 'imageWithUrl')
  }
  const badHost = lines(form.hosts).find(host => !validHost(host))
  if (badHost) add('hosts', 'host', { host: badHost })
  const env = words(form.authEnv)
  const badEnv = env.find(name => !validEnvName(name))
  if (form.auth === 'api_key' && badEnv) {
    add('authEnv', 'env', { name: badEnv })
  } else if (
    form.auth === 'api_key' &&
    env.length === 0 &&
    form.server === 'package'
  ) {
    add('authEnv', 'envRequired')
  }
  const seen = new Set<string>()
  for (const tool of form.tools) {
    const name = tool.name.trim()
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(name)) {
      add('tools', 'toolName', { name })
    } else if (seen.has(name)) {
      add('tools', 'toolTwice', { name })
    } else if (
      !tool.description.trim() ||
      tool.description.trim().length > MAX_DESCRIPTION
    ) {
      add('tools', 'toolDescription', { name, max: MAX_DESCRIPTION })
    }
    seen.add(name)
  }
  if (words(form.instructions).length > MAX_INSTRUCTION_WORDS) {
    add('instructions', 'instructions', { max: MAX_INSTRUCTION_WORDS })
  }
  return errors
}

/** The form as `scaffold_connector` takes it */
export function scaffoldInput(form: ScaffoldForm): ScaffoldConnectorInput {
  const tools: PackageTool[] = form.tools.map(tool => ({
    name: tool.name.trim(),
    description: tool.description.trim(),
    access: tool.access,
  }))
  const isPackage = form.server === 'package'
  return {
    id: form.id.trim(),
    name: form.name.trim(),
    description: form.description.trim(),
    version: form.version.trim(),
    run: isPackage ? form.run.trim() : null,
    url: isPackage ? null : form.url.trim(),
    image: isPackage && form.image.trim() ? form.image.trim() : null,
    auth: {
      kind: form.auth,
      env: form.auth === 'api_key' ? words(form.authEnv) : [],
      scopes: form.auth === 'oauth' ? words(form.authScopes) : [],
      label:
        form.auth === 'api_key' && form.authLabel.trim()
          ? form.authLabel.trim()
          : null,
    },
    hosts: lines(form.hosts),
    tools,
    reads: lines(form.reads),
    writes: lines(form.writes),
    tags: form.tags
      .split(',')
      .map(tag => tag.trim())
      .filter(Boolean),
    instructions: form.instructions.trim(),
    apiKey:
      form.auth === 'api_key' && form.apiKey.trim() ? form.apiKey.trim() : null,
  }
}

/** Registries to publish to: the person's own first, the public one last */
export function publishTargets(registries: RegistryInfo[]): RegistryInfo[] {
  return [...registries].sort(
    (a, b) => Number(a.kind === 'public') - Number(b.kind === 'public')
  )
}
