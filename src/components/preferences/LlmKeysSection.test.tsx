import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { LlmKeysSection } from './LlmKeysSection'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

describe('LlmKeysSection', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getLlmKeyStatus: ok([
        { provider: 'anthropic', configured: true },
        { provider: 'openAi', configured: true },
        { provider: 'deepSeek', configured: false },
      ]),
      setLlmApiKey: ok(null),
      testLlmKey: ok({ ok: true, latencyMs: 7, failure: null, detail: null }),
      detectOllama: ok({ running: true, models: ['llama3.2:latest'] }),
    })
  })

  it('asks before removing a key and says what calls switch to', async () => {
    const user = userEvent.setup()
    render(<LlmKeysSection />)

    const [removeAnthropic] = await screen.findAllByRole('button', {
      name: 'Remove',
    })
    await user.click(removeAnthropic as HTMLElement)
    expect(
      await screen.findByText('Remove your Anthropic key?')
    ).toBeInTheDocument()
    expect(
      screen.getByText('Agents using these models will switch to OpenAI.')
    ).toBeInTheDocument()
    expect(mocks().setLlmApiKey).not.toHaveBeenCalled()

    await user.click(screen.getByRole('button', { name: 'Remove key' }))
    await waitFor(() =>
      expect(mocks().setLlmApiKey).toHaveBeenCalledWith('anthropic', '')
    )
  })

  it('cancelling keeps the key', async () => {
    const user = userEvent.setup()
    render(<LlmKeysSection />)
    const [removeAnthropic] = await screen.findAllByRole('button', {
      name: 'Remove',
    })
    await user.click(removeAnthropic as HTMLElement)
    await user.click(await screen.findByRole('button', { name: 'Cancel' }))
    expect(mocks().setLlmApiKey).not.toHaveBeenCalled()
  })

  it('tests a stored key', async () => {
    const user = userEvent.setup()
    render(<LlmKeysSection />)
    const [testAnthropic] = await screen.findAllByRole('button', {
      name: 'Test',
    })
    await user.click(testAnthropic as HTMLElement)
    expect(
      await screen.findByText('Works. Checked in 7 ms.')
    ).toBeInTheDocument()
    expect(mocks().testLlmKey).toHaveBeenCalledWith('anthropic', null)
  })

  it('shows the free tokens left while calls use them', async () => {
    Object.assign(commands, {
      getDefaultModel: ok({
        defaultModel: 'deepseek:deepseek-flash',
        effectiveModel: 'deepseek:deepseek-flash',
        claudeSignIn: null,
        trial: { used: 25_000, limit: 100_000, enabled: true, active: true },
      }),
    })
    render(<LlmKeysSection />)
    expect(
      await screen.findByText(/In use: 75,000 of 100,000 free tokens left/)
    ).toBeInTheDocument()
  })

  it('turns the free tokens back on after the user set up their own model', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: 'claude:claude-sonnet-5',
        claudeSignIn: null,
        trial: { used: 1_000, limit: 100_000, enabled: false, active: false },
      }),
      setFreeTrial: ok({
        defaultModel: 'deepseek:deepseek-flash',
        effectiveModel: 'deepseek:deepseek-flash',
        claudeSignIn: null,
        trial: null,
      }),
    })
    render(<LlmKeysSection />)
    expect(
      await screen.findByText(/You still have 99,000 free tokens/)
    ).toBeInTheDocument()
    await user.click(screen.getByRole('switch', { name: 'Use free tokens' }))
    await waitFor(() => expect(mocks().setFreeTrial).toHaveBeenCalledWith(true))
  })

  it('can’t turn the free tokens on once they’re used up', async () => {
    Object.assign(commands, {
      getDefaultModel: ok({
        defaultModel: 'deepseek:deepseek-flash',
        effectiveModel: null,
        claudeSignIn: null,
        trial: { used: 100_000, limit: 100_000, enabled: true, active: false },
      }),
    })
    render(<LlmKeysSection />)
    expect(
      await screen.findByText(/Your 100,000 free tokens are used up/)
    ).toBeInTheDocument()
    const toggle = screen.getByRole('switch', { name: 'Use free tokens' })
    expect(toggle).toBeDisabled()
    expect(toggle).not.toBeChecked()
  })

  it('takes any model the provider has, typed, with suggestions', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      setDefaultModel: ok({
        defaultModel: 'gpt:gpt-6-preview',
        effectiveModel: 'gpt:gpt-6-preview',
        claudeSignIn: null,
        trial: null,
      }),
    })
    render(<LlmKeysSection />)
    const provider = await screen.findByRole('combobox', { name: 'Provider' })
    await user.selectOptions(provider, 'openAi')
    const model = screen.getByRole('combobox', { name: 'Model' })
    // Picking a provider fills in its usual model
    expect(model).toHaveValue('gpt-5')
    expect(screen.queryByText(/No OpenAI key yet/)).toBeNull()
    // A provider without a key says where to add one
    await user.selectOptions(provider, 'deepSeek')
    expect(model).toHaveValue('deepseek-flash')
    expect(screen.getByText(/No DeepSeek key yet/)).toBeInTheDocument()
    await user.selectOptions(provider, 'openAi')
    await user.clear(model)
    await user.type(model, 'gpt-6-preview{Enter}')
    await waitFor(() =>
      expect(mocks().setDefaultModel).toHaveBeenCalledWith('gpt:gpt-6-preview')
    )
  })

  it('suggests the provider’s own current models once its key is set', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listProviderModels: ok(['gpt-6-preview', 'gpt-5']),
    })
    const { container } = render(<LlmKeysSection />)
    await user.selectOptions(
      await screen.findByRole('combobox', { name: 'Provider' }),
      'openAi'
    )
    await waitFor(() =>
      expect(
        Array.from(
          container.querySelectorAll('#llm-model-suggestions option')
        ).map(o => o.getAttribute('value'))
      ).toEqual(['gpt-6-preview', 'gpt-5'])
    )
    expect(mocks().listProviderModels).toHaveBeenCalledWith('openAi')
    // No key: the built-in hints, and the provider isn't asked
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Provider' }),
      'deepSeek'
    )
    expect(mocks().listProviderModels).not.toHaveBeenCalledWith('deepSeek')
    expect(
      Array.from(
        container.querySelectorAll('#llm-model-suggestions option')
      ).map(o => o.getAttribute('value'))
    ).toEqual(['deepseek-flash', 'deepseek-chat'])
  })

  it('says which model calls use, and why', async () => {
    Object.assign(commands, {
      getDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: 'gpt:gpt-5',
        claudeSignIn: null,
        trial: null,
      }),
    })
    render(<LlmKeysSection />)
    expect(await screen.findByTestId('model-reason')).toHaveTextContent(
      'No Anthropic key yet, so calls use GPT-5.'
    )
  })

  it('shows a running Ollama', async () => {
    render(<LlmKeysSection />)
    expect(await screen.findByText('Running · 1 model')).toBeInTheDocument()
  })
})
