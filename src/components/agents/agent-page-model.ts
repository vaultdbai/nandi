/**
 * Pure rules for the Agents list and the agent page: runtime and model in
 * plain words, the 7-day health cells, the one problem line a row shows,
 * the list's order, the "Part of" a run belongs to, and the Settings tab's
 * provider/model choices. No React, so all of it is unit-tested.
 */
import type {
  AgentDay,
  AgentOverview,
  HarnessInfo,
  KeyedProvider,
  LlmKeyStatus,
  RunSummary,
} from '@/lib/bindings'
import { runtimeUsable } from '@/components/harness/runtime-options'
import { KEYED_PROVIDERS, modelLabel, providersWithKeys } from '@/lib/models'
import type { AgentInfo } from '@/types/agent'

/** The agent page's tabs */
export type AgentTab = 'overview' | 'chat' | 'runs' | 'settings'
export const AGENT_TABS: AgentTab[] = ['overview', 'chat', 'runs', 'settings']

/** Runtimes by their display name; unknown ones keep their ID */
const RUNTIME_NAMES: Record<string, string> = {
  native: 'Nandi',
  'claude-code': 'Claude Code',
  codex: 'Codex',
  'gemini-cli': 'Gemini CLI',
}

export function runtimeName(runtime: string): string {
  return RUNTIME_NAMES[runtime] ?? runtime.replace(/^custom:/, '')
}

export const isCliRuntime = (runtime: string) => runtime !== 'native'

/**
 * Line 2 of the Runtime column: the model for Nandi's own runtime; a CLI
 * runs in the sandbox container. `defaultModel` names the default when the
 * agent has none.
 */
export function runtimeDetail(
  overview: Pick<AgentOverview, 'runtime' | 'model'>,
  defaultModel: string | null
): { kind: 'model'; label: string } | { kind: 'container' } {
  if (isCliRuntime(overview.runtime)) return { kind: 'container' }
  const model = overview.model ?? defaultModel
  return { kind: 'model', label: model ? modelLabel(model) : '' }
}

/** How a day cell looks */
export type DayTone = 'none' | 'ok' | 'some' | 'all'

export function dayTone(day: AgentDay): DayTone {
  if (day.completed + day.failed === 0) return 'none'
  if (day.failed === 0) return 'ok'
  return day.completed === 0 ? 'all' : 'some'
}

/** The text a broken skill.md's model error starts with (domain `ModelRef`) */
const MODEL_FORMAT = /should be '<provider>:<model>'|<provider>:<model>/

/** The agent's file names a model Nandi can't read */
export function hasModelFormatError(text: string | null | undefined): boolean {
  return Boolean(text && MODEL_FORMAT.test(text))
}

/** A CLI runtime that can't sign in inside the sandbox container */
export function runtimeAuthMissing(
  overview: Pick<AgentOverview, 'runtime'> | undefined,
  harnesses: HarnessInfo[] | undefined
): boolean {
  if (!overview || !isCliRuntime(overview.runtime)) return false
  const harness = harnesses?.find(h => h.id === overview.runtime)
  if (!harness || !runtimeUsable(harness)) return false
  return !harness.health.containerAuthenticated
}

/** The one problem a list row (and the page) points at, if any */
export type AgentProblem =
  | { kind: 'waiting'; count: number }
  | { kind: 'badModel' }
  | { kind: 'broken' }
  | { kind: 'noModel' }
  | { kind: 'runtimeAuth' }

export function agentProblem(input: {
  agent: AgentInfo
  pending: number
  /** No model can be called (`DefaultModelStatus.effectiveModel === null`) */
  noModel: boolean
  /** A CLI runtime in its container that can't sign in there */
  runtimeAuthMissing: boolean
}): AgentProblem | null {
  const { agent } = input
  if (input.pending > 0) return { kind: 'waiting', count: input.pending }
  if (agent.status === 'error') {
    return hasModelFormatError(agent.description)
      ? { kind: 'badModel' }
      : { kind: 'broken' }
  }
  if (input.runtimeAuthMissing) return { kind: 'runtimeAuth' }
  if (input.noModel) return { kind: 'noModel' }
  return null
}

/**
 * Built-in agents (Chief of Staff) first, then waiting for me, then
 * problems, then the latest run, then by name
 */
export function sortAgents<
  T extends {
    name: string
    builtin?: boolean
    problem: AgentProblem | null
    lastRunAt: number | null
  },
>(rows: T[]): T[] {
  const rank = (row: T) =>
    row.builtin ? 0 : row.problem?.kind === 'waiting' ? 1 : row.problem ? 2 : 3
  return [...rows].sort(
    (a, b) =>
      rank(a) - rank(b) ||
      (b.lastRunAt ?? 0) - (a.lastRunAt ?? 0) ||
      a.name.localeCompare(b.name)
  )
}

/** The "Shared" column */
export function sharedLabel(
  agent: Pick<AgentInfo, 'origin'>
): { kind: 'from'; publisher: string } | { kind: 'none' } {
  switch (agent.origin.kind) {
    case 'publisher':
      return { kind: 'from', publisher: agent.origin.publisher }
    case 'pack':
      return { kind: 'from', publisher: agent.origin.pack }
    default:
      return { kind: 'none' }
  }
}

/** What a run was part of, before task 14 links every run to a Request */
export type RunOrigin =
  | { kind: 'chat' }
  | { kind: 'workflow'; label: string | null; workflowRunId: string | null }
  | { kind: 'request'; label: string | null }
  | { kind: 'trigger'; label: string | null }

export function runOrigin(run: RunSummary): RunOrigin {
  switch (run.sourceType) {
    case 'workflow':
      return {
        kind: 'workflow',
        label: run.sourceLabel,
        workflowRunId: run.sourceId,
      }
    case 'agent':
      return { kind: 'request', label: run.sourceLabel }
    case 'user':
      return { kind: 'chat' }
    // Schedules, files, webhooks, devices, events: what fired it
    default:
      return { kind: 'trigger', label: run.sourceLabel }
  }
}

// ---------------------------------------------------------------------------
// Settings › Model: provider and model by display name
// ---------------------------------------------------------------------------

/** skill.md's model prefix for each keyed provider */
const PREFIX: Record<KeyedProvider, string> = {
  anthropic: 'claude',
  openAi: 'gpt',
  deepSeek: 'deepseek',
}

export interface ProviderChoice {
  /** skill.md prefix: `claude`, `gpt`, `deepseek`, `ollama` */
  prefix: string
  /** `models.provider.*.name` or `models.ollama.name` */
  labelKey: string
  /** Models by `{ id, label }`; `id` is the part after the prefix */
  models: { id: string; label: string }[]
  /** A key is saved (or Ollama runs) */
  usable: boolean
}

export function providerChoices(
  keys: LlmKeyStatus[] | undefined,
  ollamaModels: string[] | null
): ProviderChoice[] {
  const withKeys = providersWithKeys(keys)
  const keyed = KEYED_PROVIDERS.map(info => ({
    prefix: PREFIX[info.provider],
    labelKey: `models.provider.${info.provider}.name`,
    models: info.models.map(model => ({
      id: model.slice(model.indexOf(':') + 1),
      label: modelLabel(model),
    })),
    usable: withKeys.includes(info.provider),
  }))
  const ollama = {
    prefix: 'ollama',
    labelKey: 'models.ollama.name',
    models: (ollamaModels ?? []).map(name => {
      const id = name.replace(/:latest$/, '')
      return { id, label: id }
    }),
    usable: ollamaModels !== null,
  }
  return [...keyed, ollama]
}

/**
 * The model list for `prefix`, keeping the saved model even when the
 * catalog doesn't list it (so opening the tab never changes it)
 */
export function modelOptions(
  choices: ProviderChoice[],
  prefix: string,
  saved: string
): { id: string; label: string }[] {
  const models = choices.find(c => c.prefix === prefix)?.models ?? []
  if (saved && !models.some(m => m.id === saved)) {
    return [...models, { id: saved, label: modelLabel(`${prefix}:${saved}`) }]
  }
  return models
}

/** Memory and CPU in sentences, or `null` when fine */
export function resourceError(
  field: 'memoryMb' | 'cpu',
  value: string
): 'memory' | 'cpu' | null {
  const text = value.trim()
  if (!text) return null
  const number = Number(text)
  if (field === 'memoryMb') {
    return Number.isInteger(number) && number >= 16 && number <= 1_048_576
      ? null
      : 'memory'
  }
  return Number.isFinite(number) && number > 0 && number <= 256 ? null : 'cpu'
}

/** New-agent starting points; each fills instructions Studio can refine */
export const AGENT_TEMPLATES = [
  'blank',
  'researcher',
  'writer',
  'helper',
] as const
export type AgentTemplate = (typeof AGENT_TEMPLATES)[number]

/** A folder ID from a name: lowercase letters, digits and hyphens */
export function agentIdFrom(name: string, taken: string[]): string {
  const base =
    name
      .normalize('NFKD')
      .replace(/[̀-ͯ]/g, '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 60)
      .replace(/-+$/, '') || 'agent'
  if (!taken.includes(base)) return base
  let n = 2
  while (taken.includes(`${base}-${n}`)) n++
  return `${base}-${n}`
}
