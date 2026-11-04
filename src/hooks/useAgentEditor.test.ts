import { describe, expect, it } from 'vitest'
import {
  isValidHost,
  parseAgentDefinition,
  toDefinitionInput,
  validateDraft,
  type EditorDraft,
} from './useAgentEditor'

const draft = (patch: Partial<EditorDraft> = {}): EditorDraft => ({
  name: 'Writer',
  description: '',
  skillBody: '',
  instructions: '',
  modelProvider: '',
  modelName: '',
  image: '',
  memoryMb: '',
  cpu: '',
  tools: [],
  network: [],
  pilots: [],
  runtime: 'native',
  runtimeModel: '',
  awsProfile: '',
  ...patch,
})

describe('validateDraft', () => {
  it('accepts a minimal draft', () => {
    expect(validateDraft(draft())).toEqual({})
  })

  it('requires a name', () => {
    expect(validateDraft(draft({ name: '  ' })).name).toBe(
      'editor.errors.nameRequired'
    )
    expect(validateDraft(draft({ name: 'x'.repeat(201) })).name).toBe(
      'editor.errors.tooLong'
    )
  })

  it('needs a provider and a model name together', () => {
    expect(validateDraft(draft({ modelProvider: 'claude' })).model).toBe(
      'editor.errors.modelNameRequired'
    )
    expect(validateDraft(draft({ modelName: 'x' })).model).toBe(
      'editor.errors.modelProvider'
    )
    expect(
      validateDraft(draft({ modelProvider: 'gemini', modelName: 'pro' })).model
    ).toBe('editor.errors.modelProvider')
    expect(
      validateDraft(draft({ modelProvider: 'claude', modelName: 'a b' })).model
    ).toBe('editor.errors.noSpaces')
    expect(
      validateDraft(
        draft({ modelProvider: 'claude', modelName: 'claude-sonnet-5' })
      )
    ).toEqual({})
  })

  it('checks container limits', () => {
    expect(validateDraft(draft({ memoryMb: '8' })).memoryMb).toBeDefined()
    expect(validateDraft(draft({ memoryMb: '1.5' })).memoryMb).toBeDefined()
    expect(validateDraft(draft({ memoryMb: '2048' })).memoryMb).toBeUndefined()
    expect(validateDraft(draft({ cpu: '0' })).cpu).toBeDefined()
    expect(validateDraft(draft({ cpu: '300' })).cpu).toBeDefined()
    expect(validateDraft(draft({ cpu: 'abc' })).cpu).toBeDefined()
    expect(validateDraft(draft({ cpu: '1.5' })).cpu).toBeUndefined()
    expect(validateDraft(draft({ image: 'a b' })).image).toBeDefined()
  })

  it('checks hosts and duplicate pilots', () => {
    expect(
      validateDraft(draft({ network: ['api.github.com', 'a.*.com'] })).network
    ).toBe('editor.errors.invalidHost')
    expect(validateDraft(draft({ pilots: ['b', 'b'] })).pilots).toBe(
      'editor.errors.duplicatePilot'
    )
  })
})

describe('isValidHost', () => {
  it('matches the backend rules', () => {
    for (const host of ['api.example.com', '*.example.com', '*', 'x.io:8443'])
      expect(isValidHost(host), host).toBe(true)
    for (const host of ['', 'a.*.com', '*x.com', 'host:99999', 'a b'])
      expect(isValidHost(host), host).toBe(false)
  })
})

describe('toDefinitionInput', () => {
  it('trims and converts the draft', () => {
    const input = toDefinitionInput(
      draft({
        name: ' Writer ',
        modelProvider: 'claude',
        modelName: 'claude-sonnet-5',
        memoryMb: '2048',
        cpu: '1.5',
        tools: ['shell', ' '],
      })
    )
    expect(input).toMatchObject({
      name: 'Writer',
      modelProvider: 'claude',
      image: null,
      memoryMb: 2048,
      cpu: 1.5,
      capabilities: { tools: ['shell'], network: [], pilots: [] },
    })
  })
})

describe('parseAgentDefinition', () => {
  it('reads what loadEntity returns', () => {
    const definition = parseAgentDefinition({
      id: 'writer',
      name: 'Writer',
      skillBody: 'Write.',
      memoryMb: 2048,
      cpu: null,
      capabilities: { tools: ['shell'], network: ['a.io'], pilots: [] },
      readOnly: false,
      allowedTools: null,
      requiresApproval: ['shell', 'write_file'],
      networkRules: [{ host: 'a.io', methods: ['GET'] }],
      skillMd: '---\nname: Writer\n---\n',
    })
    expect(definition.draft).toMatchObject({
      name: 'Writer',
      skillBody: 'Write.',
      memoryMb: '2048',
      cpu: '',
      tools: ['shell'],
    })
    expect(definition.allowedTools).toBeNull()
    expect(definition.networkRules[0]?.methods).toEqual(['GET'])
    expect(() => parseAgentDefinition('nope')).toThrow()
  })
})

describe('runtime', () => {
  it('a harness runtime is saved with its model', () => {
    const harness = draft({ runtime: 'claude-code', runtimeModel: ' sonnet ' })
    const input = toDefinitionInput(harness)
    expect(input.runtime).toBe('claude-code')
    expect(input.runtimeModel).toBe('sonnet')
  })

  it('native saves no runtime model', () => {
    const native = draft({ runtime: 'native', runtimeModel: 'sonnet' })
    expect(toDefinitionInput(native).runtimeModel).toBeNull()
  })

  it('reads the runtime from a loaded definition, native by default', () => {
    expect(parseAgentDefinition({ id: 'a', name: 'A' }).draft.runtime).toBe(
      'native'
    )
    const loaded = parseAgentDefinition({
      id: 'a',
      name: 'A',
      runtime: 'gemini-cli',
      runtimeModel: 'gemini-2.5-pro',
    })
    expect(loaded.draft.runtime).toBe('gemini-cli')
    expect(loaded.draft.runtimeModel).toBe('gemini-2.5-pro')
  })
})

describe('AWS profile', () => {
  it('is read from the definition and saved trimmed, empty as the default', () => {
    const loaded = parseAgentDefinition({
      id: 'ops',
      name: 'Ops',
      awsProfile: 'work',
      hasAws: true,
    })
    expect(loaded.draft.awsProfile).toBe('work')
    expect(loaded.hasAws).toBe(true)
    expect(parseAgentDefinition({ id: 'a', name: 'A' }).hasAws).toBe(false)

    expect(toDefinitionInput(draft({ awsProfile: ' work ' })).awsProfile).toBe(
      'work'
    )
    expect(toDefinitionInput(draft({ awsProfile: ' ' })).awsProfile).toBeNull()
  })
})
