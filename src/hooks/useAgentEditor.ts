/**
 * An agent's definition as stored in its files (gold copy) and saving it
 * back, for the agent page's Settings tab. Saves go through `repoSaveAgent`,
 * which writes the files; nothing here touches the projection.
 *
 * Also the pure helpers (parsing, validation), so they're testable
 * without React.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { withSite } from '@/components/agents/network-access-model'
import { agentsQueryKeys } from '@/hooks/useAgents'
import { client } from '@/lib/client'
import { logger } from '@/lib/logger'
import type {
  AgentDefinitionInput,
  AgentInfo,
  HostRule,
  JsonValue,
  PilotDefinition,
} from '@/lib/tauri-bindings'

const editorQueryKeys = {
  definition: (agentId: string) => ['agent-definition', agentId] as const,
  pilots: ['pilot-definitions'] as const,
}

// ---------------------------------------------------------------------------
// The definition
// ---------------------------------------------------------------------------

/** Model routing prefixes (see reserved-conventions.md) */
const MODEL_PROVIDERS = ['deepseek', 'claude', 'gpt', 'ollama'] as const

/** Everything the Settings tab edits, with every field present */
export interface EditorDraft {
  name: string
  description: string
  skillBody: string
  instructions: string
  /** `''`: the settings default model */
  modelProvider: string
  modelName: string
  image: string
  /** Kept as typed; validated and converted on save */
  memoryMb: string
  cpu: string
  tools: string[]
  network: string[]
  pilots: string[]
  /** `native` or a harness (`claude-code`, `codex`, …) */
  runtime: string
  /** The harness's model; `''`: its default */
  runtimeModel: string
  /** `aws.profile`; `''`: Settings' access key, else the sign-in profile */
  awsProfile: string
}

/** An agent as `repoLoadEntity('agent', id)` returns it */
export interface AgentDefinition {
  id: string
  draft: EditorDraft
  /** Marketplace agents can't be saved; fork them first */
  readOnly: boolean
  /** An installed agent running with the user's own instructions */
  instructionsOverridden: boolean
  /** `null`: every runtime tool */
  allowedTools: string[] | null
  /** Tools that need approval, with the default applied */
  requiresApproval: string[]
  networkRules: HostRule[]
  /** skill.md has an `aws` section */
  hasAws: boolean
  workflows: string[]
  /** skill.md as it is on disk */
  skillMd: string
  /** What saving renders for the agent unchanged */
  renderedSkillMd: string
  sourcePath: string
}

type JsonObject = Partial<Record<string, JsonValue>>

const isObject = (value: JsonValue | undefined): value is JsonObject =>
  typeof value === 'object' && value !== null && !Array.isArray(value)

const str = (value: JsonValue | undefined): string =>
  typeof value === 'string' ? value : ''

const num = (value: JsonValue | undefined): string =>
  typeof value === 'number' ? String(value) : ''

const strings = (value: JsonValue | undefined): string[] =>
  Array.isArray(value)
    ? value.filter((v): v is string => typeof v === 'string')
    : []

export function parseAgentDefinition(value: JsonValue): AgentDefinition {
  if (!isObject(value)) throw new Error('Unexpected agent definition')
  const capabilities = isObject(value.capabilities) ? value.capabilities : {}
  const rules = Array.isArray(value.networkRules) ? value.networkRules : []
  return {
    id: str(value.id),
    draft: {
      name: str(value.name),
      description: str(value.description),
      skillBody: str(value.skillBody),
      instructions: str(value.instructions),
      modelProvider: str(value.modelProvider),
      modelName: str(value.modelName),
      image: str(value.image),
      memoryMb: num(value.memoryMb),
      cpu: num(value.cpu),
      tools: strings(capabilities.tools),
      network: strings(capabilities.network),
      pilots: strings(capabilities.pilots),
      runtime: str(value.runtime) || 'native',
      runtimeModel: str(value.runtimeModel),
      awsProfile: str(value.awsProfile),
    },
    readOnly: value.readOnly === true,
    instructionsOverridden: value.instructionsOverridden === true,
    allowedTools: Array.isArray(value.allowedTools)
      ? strings(value.allowedTools)
      : null,
    requiresApproval: strings(value.requiresApproval),
    networkRules: rules.filter(isObject).map(rule => ({
      host: str(rule.host),
      methods: strings(rule.methods),
    })),
    hasAws: value.hasAws === true,
    workflows: strings(value.workflows),
    skillMd: str(value.skillMd),
    renderedSkillMd: str(value.renderedSkillMd),
    sourcePath: str(value.sourcePath),
  }
}

const optional = (value: string): string | null => value.trim() || null

const clean = (items: string[]) =>
  items.map(item => item.trim()).filter(Boolean)

/** What `repoSaveAgent` takes. Call after `validateDraft` finds no errors. */
export function toDefinitionInput(draft: EditorDraft): AgentDefinitionInput {
  return {
    name: draft.name.trim(),
    description: draft.description.trim(),
    skillBody: draft.skillBody,
    instructions: draft.instructions,
    modelProvider: optional(draft.modelProvider),
    modelName: optional(draft.modelName),
    image: optional(draft.image),
    memoryMb: draft.memoryMb.trim() ? Number(draft.memoryMb) : null,
    cpu: draft.cpu.trim() ? Number(draft.cpu) : null,
    capabilities: {
      tools: clean(draft.tools),
      network: clean(draft.network),
      pilots: clean(draft.pilots),
    },
    runtime: optional(draft.runtime),
    runtimeModel:
      draft.runtime === 'native' ? null : optional(draft.runtimeModel),
    awsProfile: optional(draft.awsProfile),
  }
}

// ---------------------------------------------------------------------------
// Validation (mirrors the domain's rules, so most errors show before saving)
// ---------------------------------------------------------------------------

type DraftField =
  | 'name'
  | 'description'
  | 'skillBody'
  | 'instructions'
  | 'model'
  | 'image'
  | 'memoryMb'
  | 'cpu'
  | 'network'
  | 'pilots'

/** Field → an `editor.errors.*` translation key */
type DraftErrors = Partial<Record<DraftField, string>>

const MAX_NAME_CHARS = 200
const MAX_DESCRIPTION_CHARS = 2_000
const MAX_PROMPT_CHARS = 200_000

const HOST_NAME = /^(\*|(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)*)$/

/** `api.example.com`, `*.example.com`, or `*`, with an optional `:port` */
export function isValidHost(host: string): boolean {
  const normalized = host.trim().replace(/\.$/, '').toLowerCase()
  const colon = normalized.lastIndexOf(':')
  const name = colon === -1 ? normalized : normalized.slice(0, colon)
  const port = colon === -1 ? null : normalized.slice(colon + 1)
  if (port !== null) {
    const value = Number(port)
    if (!/^\d+$/.test(port) || value > 65_535) return false
  }
  return HOST_NAME.test(name)
}

export function validateDraft(draft: EditorDraft): DraftErrors {
  const errors: DraftErrors = {}
  const name = draft.name.trim()
  if (!name) errors.name = 'editor.errors.nameRequired'
  else if (name.length > MAX_NAME_CHARS) errors.name = 'editor.errors.tooLong'
  if (draft.description.trim().length > MAX_DESCRIPTION_CHARS)
    errors.description = 'editor.errors.tooLong'
  if (draft.skillBody.trim().length > MAX_PROMPT_CHARS)
    errors.skillBody = 'editor.errors.tooLong'
  if (draft.instructions.trim().length > MAX_PROMPT_CHARS)
    errors.instructions = 'editor.errors.tooLong'

  const provider = draft.modelProvider.trim()
  const model = draft.modelName.trim()
  if (provider && !(MODEL_PROVIDERS as readonly string[]).includes(provider))
    errors.model = 'editor.errors.modelProvider'
  else if (provider && !model) errors.model = 'editor.errors.modelNameRequired'
  else if (!provider && model) errors.model = 'editor.errors.modelProvider'
  else if (/\s/.test(model)) errors.model = 'editor.errors.noSpaces'

  if (/\s/.test(draft.image.trim())) errors.image = 'editor.errors.noSpaces'

  const memory = draft.memoryMb.trim()
  if (memory) {
    const value = Number(memory)
    if (!Number.isInteger(value) || value < 16 || value > 1_048_576)
      errors.memoryMb = 'editor.errors.memoryRange'
  }
  const cpu = draft.cpu.trim()
  if (cpu) {
    const value = Number(cpu)
    if (!Number.isFinite(value) || value <= 0 || value > 256)
      errors.cpu = 'editor.errors.cpuRange'
  }

  if (clean(draft.network).some(host => !isValidHost(host)))
    errors.network = 'editor.errors.invalidHost'
  const pilots = clean(draft.pilots)
  if (new Set(pilots).size !== pilots.length)
    errors.pilots = 'editor.errors.duplicatePilot'
  return errors
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

async function unwrap<T>(
  promise: Promise<
    { status: 'ok'; data: T } | { status: 'error'; error: string }
  >,
  what: string
): Promise<T> {
  const result = await promise
  if (result.status === 'error') {
    logger.error(what, { error: result.error })
    throw new Error(result.error)
  }
  return result.data
}

const loadDefinition = async (agentId: string) =>
  parseAgentDefinition(
    await unwrap(
      client.repoLoadEntity('agent', agentId),
      'Failed to load the agent definition'
    )
  )

/** The agent's definition, read from its files */
export function useAgentDefinition(agentId: string | undefined) {
  return useQuery({
    queryKey: editorQueryKeys.definition(agentId ?? ''),
    queryFn: () => loadDefinition(agentId ?? ''),
    enabled: Boolean(agentId),
  })
}

/** After the repo's files change: what the agent page shows */
function useInvalidateRepo() {
  const queryClient = useQueryClient()
  return (agentId?: string) => {
    void queryClient.invalidateQueries({ queryKey: agentsQueryKeys.all })
    void queryClient.invalidateQueries({ queryKey: ['git-status'] })
    if (agentId)
      void queryClient.invalidateQueries({
        queryKey: editorQueryKeys.definition(agentId),
      })
  }
}

/** Writes the definition to the agent's files (gold copy) */
export function useSaveAgent(agentId: string) {
  const invalidate = useInvalidateRepo()
  return useMutation({
    mutationFn: (input: AgentDefinitionInput): Promise<AgentInfo> =>
      unwrap(client.repoSaveAgent(agentId, input), 'Failed to save the agent'),
    onSuccess: () => invalidate(agentId),
  })
}

/**
 * Adds one site to the agent's allowed sites and saves its files, keeping
 * everything else as it is on disk (e.g. Allow on a blocked request)
 */
export function useAllowSite(agentId: string) {
  const invalidate = useInvalidateRepo()
  return useMutation({
    mutationFn: async (site: string): Promise<AgentInfo> => {
      const { draft } = await loadDefinition(agentId)
      return unwrap(
        client.repoSaveAgent(
          agentId,
          toDefinitionInput({
            ...draft,
            network: withSite(draft.network, site),
          })
        ),
        'Failed to allow the site'
      )
    },
    onSuccess: () => invalidate(agentId),
  })
}

/** Replaces the agent's instructions (an installed agent's: an override) */
export function useSaveAgentInstructions(agentId: string) {
  const invalidate = useInvalidateRepo()
  return useMutation({
    mutationFn: (instructions: string) =>
      unwrap(
        client.repoSaveAgentInstructions(agentId, instructions),
        'Failed to save the rules'
      ),
    onSuccess: () => invalidate(agentId),
  })
}

/** Back to an installed agent's shipped instructions */
export function useResetAgentInstructions(agentId: string) {
  const invalidate = useInvalidateRepo()
  return useMutation({
    mutationFn: () =>
      unwrap(
        client.repoResetAgentInstructions(agentId),
        'Failed to reset the rules'
      ),
    onSuccess: () => invalidate(agentId),
  })
}

/** Moves the agent's folder to the trash */
export function useDeleteAgent() {
  const invalidate = useInvalidateRepo()
  return useMutation({
    mutationFn: (agentId: string) =>
      unwrap(client.deleteAgent(agentId), 'Failed to delete the agent'),
    onSuccess: () => invalidate(),
  })
}

/**
 * Pilots this install knows. Fails (and stays failed) until pilot support
 * ships, which callers show as "not available yet".
 */
export function usePilotDefinitions() {
  return useQuery({
    queryKey: editorQueryKeys.pilots,
    queryFn: (): Promise<PilotDefinition[]> =>
      unwrap(client.listPilots(), 'Pilots are unavailable'),
    retry: false,
    staleTime: 60_000,
  })
}
