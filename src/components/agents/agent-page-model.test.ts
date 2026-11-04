import { describe, expect, it } from 'vitest'
import type { AgentOverview, HarnessInfo, LlmKeyStatus } from '@/lib/bindings'
import { agentInfo, run } from '@/test/fixtures'
import {
  agentIdFrom,
  agentProblem,
  dayTone,
  hasModelFormatError,
  modelOptions,
  providerChoices,
  resourceError,
  runOrigin,
  runtimeAuthMissing,
  runtimeDetail,
  runtimeName,
  sharedLabel,
  sortAgents,
} from './agent-page-model'

const overview = (patch: Partial<AgentOverview> = {}): AgentOverview => ({
  agentId: 'writer',
  runtime: 'native',
  model: null,
  acceptsRequests: false,
  runs: 0,
  completed: 0,
  failed: 0,
  avgDurationMs: null,
  costUsd: 0,
  days: [],
  lastRunAt: null,
  ...patch,
})

describe('runtime in plain words', () => {
  it('names runtimes, never their IDs', () => {
    expect(runtimeName('native')).toBe('Nandi')
    expect(runtimeName('claude-code')).toBe('Claude Code')
    expect(runtimeName('gemini-cli')).toBe('Gemini CLI')
    expect(runtimeName('custom:mine')).toBe('mine')
  })

  it('shows the model for Nandi and where a CLI runs', () => {
    expect(
      runtimeDetail(overview({ model: 'claude:claude-sonnet-5' }), null)
    ).toEqual({ kind: 'model', label: 'Claude Sonnet 5' })
    // No model of its own: the default, by name
    expect(runtimeDetail(overview(), 'gpt:gpt-5')).toEqual({
      kind: 'model',
      label: 'GPT-5',
    })
    expect(runtimeDetail(overview({ runtime: 'codex' }), null)).toEqual({
      kind: 'container',
    })
  })
})

describe('dayTone', () => {
  it('colours a day by how its runs ended', () => {
    expect(dayTone({ completed: 0, failed: 0 })).toBe('none')
    expect(dayTone({ completed: 3, failed: 0 })).toBe('ok')
    expect(dayTone({ completed: 2, failed: 1 })).toBe('some')
    expect(dayTone({ completed: 0, failed: 2 })).toBe('all')
  })
})

describe('agentProblem', () => {
  const ok = agentInfo('writer', 'Writer')
  const base = {
    agent: ok,
    pending: 0,
    noModel: false,
    runtimeAuthMissing: false,
  }

  it('puts what waits on me first', () => {
    expect(agentProblem({ ...base, pending: 2, noModel: true })).toEqual({
      kind: 'waiting',
      count: 2,
    })
  })

  it('maps a broken model line to "pick a model"', () => {
    const broken = agentInfo('writer', 'Writer', {
      status: 'error',
      description:
        "Invalid skill.md: 'claude-sonnet-4-5' should be '<provider>:<model>'",
    })
    expect(agentProblem({ ...base, agent: broken })).toEqual({
      kind: 'badModel',
    })
    expect(
      agentProblem({
        ...base,
        agent: agentInfo('writer', 'Writer', {
          status: 'error',
          description: 'x',
        }),
      })
    ).toEqual({ kind: 'broken' })
  })

  it('then runtime sign-in, then no model, else nothing', () => {
    expect(
      agentProblem({ ...base, runtimeAuthMissing: true, noModel: true })
    ).toEqual({
      kind: 'runtimeAuth',
    })
    expect(agentProblem({ ...base, noModel: true })).toEqual({
      kind: 'noModel',
    })
    expect(agentProblem(base)).toBeNull()
  })
})

describe('hasModelFormatError', () => {
  it('spots the domain and LLM messages', () => {
    expect(hasModelFormatError("'x' should be '<provider>:<model>'")).toBe(true)
    expect(
      hasModelFormatError("Invalid model 'x': expected '<provider>:<model>'")
    ).toBe(true)
    expect(hasModelFormatError('network down')).toBe(false)
    expect(hasModelFormatError(null)).toBe(false)
  })
})

describe('sortAgents', () => {
  it('waiting, then problems, then latest run, then name', () => {
    const rows = [
      { name: 'b', problem: null, lastRunAt: 1 },
      { name: 'a', problem: null, lastRunAt: 1 },
      { name: 'late', problem: null, lastRunAt: 9 },
      { name: 'broken', problem: { kind: 'broken' as const }, lastRunAt: null },
      {
        name: 'waits',
        problem: { kind: 'waiting' as const, count: 1 },
        lastRunAt: null,
      },
    ]
    expect(sortAgents(rows).map(r => r.name)).toEqual([
      'waits',
      'broken',
      'late',
      'a',
      'b',
    ])
  })

  it('puts a built-in agent first, ahead of one waiting', () => {
    const rows = [
      {
        name: 'waits',
        problem: { kind: 'waiting' as const, count: 1 },
        lastRunAt: null,
      },
      { name: 'Chief of Staff', builtin: true, problem: null, lastRunAt: null },
    ]
    expect(sortAgents(rows).map(r => r.name)).toEqual([
      'Chief of Staff',
      'waits',
    ])
  })
})

describe('sharedLabel', () => {
  it('names where an installed agent came from', () => {
    expect(
      sharedLabel({ origin: { kind: 'publisher', publisher: 'acme' } })
    ).toEqual({ kind: 'from', publisher: 'acme' })
    expect(sharedLabel({ origin: { kind: 'local' } })).toEqual({ kind: 'none' })
  })
})

describe('runOrigin', () => {
  it('labels chat runs Chat and keeps workflow and request links', () => {
    expect(runOrigin(run('r1', 'writer', { sourceType: 'user' }))).toEqual({
      kind: 'chat',
    })
    expect(
      runOrigin(
        run('r2', 'writer', {
          sourceType: 'workflow',
          sourceId: 'wr-1',
          sourceLabel: 'Daily · step 2',
        })
      )
    ).toEqual({
      kind: 'workflow',
      label: 'Daily · step 2',
      workflowRunId: 'wr-1',
    })
    expect(runOrigin(run('r3', 'writer', { sourceType: 'agent' })).kind).toBe(
      'request'
    )
    expect(runOrigin(run('r4', 'writer', { sourceType: 'cron' })).kind).toBe(
      'trigger'
    )
  })
})

describe('provider and model choices', () => {
  const keys: LlmKeyStatus[] = [
    { provider: 'anthropic', configured: true },
    { provider: 'openAi', configured: false },
    { provider: 'deepSeek', configured: false },
  ]

  it('names providers and models, enabling only usable ones', () => {
    const choices = providerChoices(keys, ['llama3.2:latest'])
    expect(choices.map(c => [c.prefix, c.usable])).toEqual([
      ['claude', true],
      ['gpt', false],
      ['deepseek', false],
      ['ollama', true],
    ])
    expect(choices[0]?.models[0]).toEqual({
      id: 'claude-sonnet-5',
      label: 'Claude Sonnet 5',
    })
    expect(choices[3]?.models).toEqual([{ id: 'llama3.2', label: 'llama3.2' }])
    // Ollama not running
    expect(providerChoices(keys, null).at(-1)?.usable).toBe(false)
  })

  it('keeps a saved model the catalog doesn’t list', () => {
    const choices = providerChoices(keys, null)
    const models = modelOptions(choices, 'claude', 'claude-haiku-4')
    expect(models.at(-1)).toEqual({
      id: 'claude-haiku-4',
      label: 'claude-haiku-4',
    })
    expect(modelOptions(choices, 'claude', 'claude-sonnet-5')).toHaveLength(2)
  })
})

describe('resourceError', () => {
  it('checks memory and CPUs like the domain', () => {
    expect(resourceError('memoryMb', '')).toBeNull()
    expect(resourceError('memoryMb', '2048')).toBeNull()
    expect(resourceError('memoryMb', '8')).toBe('memory')
    expect(resourceError('memoryMb', '1.5')).toBe('memory')
    expect(resourceError('cpu', '0')).toBe('cpu')
    expect(resourceError('cpu', '1.5')).toBeNull()
  })
})

describe('agentIdFrom', () => {
  it('derives a free folder ID from a name', () => {
    expect(agentIdFrom('Research Assistant', [])).toBe('research-assistant')
    expect(agentIdFrom('Zoë!', [])).toBe('zoe')
    expect(agentIdFrom('Writer', ['writer', 'writer-2'])).toBe('writer-3')
    expect(agentIdFrom('???', [])).toBe('agent')
  })
})

describe('runtimeAuthMissing', () => {
  const harness = (patch: Partial<HarnessInfo['health']>): HarnessInfo =>
    ({
      id: 'claude-code',
      displayName: 'Claude Code',
      imageAvailable: false,
      health: {
        installed: true,
        version: null,
        hostAuthenticated: true,
        containerAuthenticated: false,
        ...patch,
      },
    }) as unknown as HarnessInfo

  it('checks the sign-in inside the container', () => {
    const cli = overview({ runtime: 'claude-code' })
    expect(runtimeAuthMissing(cli, [harness({})])).toBe(true)
    expect(
      runtimeAuthMissing(cli, [harness({ containerAuthenticated: true })])
    ).toBe(false)
    // Nandi's own runtime, or a runtime this computer doesn't have
    expect(runtimeAuthMissing(overview(), [harness({})])).toBe(false)
    expect(runtimeAuthMissing(cli, [harness({ installed: false })])).toBe(false)
  })
})
