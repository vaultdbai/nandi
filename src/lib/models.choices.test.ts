import { describe, expect, it } from 'vitest'
import type { LlmKeyStatus } from '@/lib/bindings'
import {
  defaultModelName,
  joinModel,
  modelReason,
  modelSuggestions,
  providerNeedsKey,
  providerOf,
  splitModel,
} from './models'

const keys = (...configured: LlmKeyStatus['provider'][]): LlmKeyStatus[] =>
  (['anthropic', 'openAi', 'deepSeek'] as const).map(provider => ({
    provider,
    configured: configured.includes(provider),
  })) as LlmKeyStatus[]

describe('a model as provider and name', () => {
  it('splits and joins the ref the backend takes', () => {
    expect(splitModel('claude:claude-sonnet-5')).toEqual({
      provider: 'anthropic',
      name: 'claude-sonnet-5',
    })
    expect(splitModel('ollama:qwen3:8b')).toEqual({
      provider: 'ollama',
      name: 'qwen3:8b',
    })
    expect(splitModel('no-prefix')).toEqual({
      provider: null,
      name: 'no-prefix',
    })
    // Any name the provider takes, not only ones Nandi knows
    expect(joinModel('openAi', ' gpt-6-preview ')).toBe('gpt:gpt-6-preview')
  })

  it('starts each provider on its usual model, Ollama on what it has', () => {
    expect(defaultModelName('anthropic', [])).toBe('claude-sonnet-5')
    expect(defaultModelName('ollama', ['qwen3:latest'])).toBe('qwen3')
    expect(defaultModelName('ollama', [])).toBe('llama3.2')
  })

  it('suggests names without limiting them', () => {
    expect(modelSuggestions('deepSeek', [])).toEqual([
      'deepseek-flash',
      'deepseek-chat',
    ])
    expect(modelSuggestions('ollama', ['llama3.2:latest', 'qwen3:8b'])).toEqual(
      ['llama3.2', 'qwen3:8b']
    )
  })

  it('knows which providers still need a key', () => {
    expect(providerNeedsKey('openAi', keys('openAi'), false)).toBe(false)
    expect(providerNeedsKey('deepSeek', keys('openAi'), false)).toBe(true)
    expect(providerNeedsKey('anthropic', keys(), true)).toBe(false)
    expect(providerNeedsKey('ollama', keys(), false)).toBe(false)
  })
})

describe('modelReason', () => {
  it('says nothing when calls use the chosen model', () => {
    expect(
      modelReason({
        defaultModel: 'gpt:gpt-5',
        effectiveModel: 'gpt:gpt-5',
        onFreeTokens: false,
      })
    ).toBeNull()
  })

  it('names the free tokens, a missing key, or no model at all', () => {
    expect(
      modelReason({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: 'deepseek:deepseek-flash',
        onFreeTokens: true,
      })
    ).toEqual({ key: 'freeTokens', model: 'deepseek:deepseek-flash' })
    expect(
      modelReason({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: 'gpt:gpt-5',
        onFreeTokens: false,
      })
    ).toEqual({ key: 'noKeyFor', provider: 'anthropic', model: 'gpt:gpt-5' })
    expect(
      modelReason({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: null,
        onFreeTokens: false,
      })
    ).toEqual({ key: 'noModel' })
  })

  it('knows each prefix’s provider', () => {
    expect(providerOf('claude:x')).toBe('anthropic')
    expect(providerOf('gpt:x')).toBe('openAi')
    expect(providerOf('deepseek:x')).toBe('deepSeek')
    expect(providerOf('ollama:x')).toBe('ollama')
    expect(providerOf('mystery')).toBeNull()
  })
})
