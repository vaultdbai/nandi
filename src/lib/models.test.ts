import { describe, expect, it } from 'vitest'
import type { LlmKeyStatus } from '@/lib/bindings'
import {
  fallbackAfterRemoving,
  keyTestOutcome,
  modelLabel,
  ollamaModelRef,
  providerInfo,
  providersWithKeys,
} from './models'

const keys = (...configured: LlmKeyStatus['provider'][]): LlmKeyStatus[] =>
  (['deepSeek', 'anthropic', 'openAi'] as const).map(provider => ({
    provider,
    configured: configured.includes(provider),
  }))

describe('providersWithKeys', () => {
  it('lists configured providers in the backend fallback order', () => {
    expect(providersWithKeys(keys('deepSeek', 'openAi', 'anthropic'))).toEqual([
      'anthropic',
      'openAi',
      'deepSeek',
    ])
    expect(providersWithKeys(keys())).toEqual([])
    expect(providersWithKeys(undefined)).toEqual([])
  })
})

describe('provider defaults', () => {
  it('never makes DeepSeek the first choice', () => {
    expect(providersWithKeys(keys('deepSeek', 'anthropic'))[0]).toBe(
      'anthropic'
    )
    expect(providerInfo('anthropic').defaultModel).toBe(
      'claude:claude-sonnet-5'
    )
  })
})

describe('modelLabel', () => {
  it('names known models in plain words', () => {
    expect(modelLabel('claude:claude-sonnet-5')).toBe('Claude Sonnet 5')
    expect(modelLabel('gpt:gpt-5')).toBe('GPT-5')
  })

  it('drops the provider prefix of other models', () => {
    expect(modelLabel('ollama:llama3.2')).toBe('llama3.2')
    expect(modelLabel('no-prefix')).toBe('no-prefix')
  })
})

describe('ollamaModelRef', () => {
  it('drops the default tag', () => {
    expect(ollamaModelRef('llama3.2:latest')).toBe('ollama:llama3.2')
    expect(ollamaModelRef('qwen3:8b')).toBe('ollama:qwen3:8b')
  })
})

describe('fallbackAfterRemoving', () => {
  it('names what calls switch to, or nothing', () => {
    expect(
      fallbackAfterRemoving('anthropic', keys('anthropic', 'openAi'), false)
    ).toBe('openAi')
    expect(fallbackAfterRemoving('anthropic', keys('anthropic'), true)).toBe(
      'ollama'
    )
    expect(fallbackAfterRemoving('anthropic', keys('anthropic'), false)).toBe(
      null
    )
  })
})

describe('keyTestOutcome', () => {
  it('maps a check to one outcome', () => {
    const test = { latencyMs: 1, detail: null }
    expect(keyTestOutcome({ ...test, ok: true, failure: null })).toBe('works')
    expect(keyTestOutcome({ ...test, ok: false, failure: 'noQuota' })).toBe(
      'noQuota'
    )
    expect(keyTestOutcome({ ...test, ok: false, failure: null })).toBe('other')
  })
})
