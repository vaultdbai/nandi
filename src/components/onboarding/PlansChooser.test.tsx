import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { render, screen } from '@/test/test-utils'
import { PlansChooser } from './PlansChooser'

function setup(trialOffered = true) {
  const onPickProvider = vi.fn()
  const onPickOllama = vi.fn()
  render(
    <PlansChooser
      trialOffered={trialOffered}
      onPickProvider={onPickProvider}
      onPickOllama={onPickOllama}
    />
  )
  return { onPickProvider, onPickOllama }
}

const open = () =>
  userEvent.click(screen.getByRole('button', { name: /already use/i }))

describe('PlansChooser', () => {
  beforeEach(() => vi.mocked(openUrl).mockClear())

  it('is collapsed until opened', async () => {
    setup()
    expect(screen.queryByRole('radio')).not.toBeInTheDocument()
    await open()
    expect(screen.getAllByRole('radio')).toHaveLength(6)
  })

  it('says a ChatGPT subscription cannot be used and opens the key page', async () => {
    const { onPickProvider } = setup()
    await open()
    await userEvent.click(screen.getByRole('radio', { name: /ChatGPT/ }))
    expect(screen.getByTestId('plan-steps')).toHaveTextContent(
      /can’t be used by other apps/
    )
    await userEvent.click(
      screen.getByRole('button', { name: 'Open OpenAI API keys' })
    )
    expect(openUrl).toHaveBeenCalledWith('https://platform.openai.com/api-keys')
    await userEvent.click(
      screen.getByRole('button', { name: 'Open the OpenAI key form' })
    )
    expect(onPickProvider).toHaveBeenCalledWith('openAi')
  })

  it('keeps Claude to Claude Code agents', async () => {
    setup()
    await open()
    await userEvent.click(screen.getByRole('radio', { name: /Claude Pro/ }))
    const steps = screen.getByTestId('plan-steps')
    expect(steps).toHaveTextContent(/Claude Code agents/)
    expect(steps).toHaveTextContent(/own replies don’t use your plan/)
  })

  it('offers no Gemini key form, and hides free tokens when not offered', async () => {
    setup(false)
    await open()
    await userEvent.click(screen.getByRole('radio', { name: /Gemini/ }))
    const steps = screen.getByTestId('plan-steps')
    expect(steps).toHaveTextContent(/doesn’t support Gemini keys yet/)
    expect(steps).not.toHaveTextContent(/free tokens/)
    expect(screen.queryByText(/key form/)).not.toBeInTheDocument()
  })

  it('opens the Ollama setup from "Nothing yet"', async () => {
    const { onPickOllama } = setup()
    await open()
    await userEvent.click(screen.getByRole('radio', { name: /Nothing yet/ }))
    await userEvent.click(screen.getByRole('button', { name: 'Set up Ollama' }))
    expect(onPickOllama).toHaveBeenCalled()
  })
})
