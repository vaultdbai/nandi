import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { streamAsksThroughAsk } from '@/test/ask-stream'
import { render, screen, waitFor } from '@/test/test-utils'
import { AskBox } from '@/components/ask/AskBox'
import type { AskResponse, HelpAnswer, HelpTopic } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { useUIStore } from '@/store/ui-store'
import { HelpGuideDialog } from './HelpGuideDialog'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    ask: vi.fn(),
    getDefaultModel: vi.fn(),
    helpAnswer: vi.fn(),
    helpTopics: vi.fn(),
    openExternal: vi.fn(),
    supports: { attachments: false },
    subscribe: vi.fn(() => ({
      ready: Promise.resolve(),
      unsubscribe: vi.fn(),
    })),
  },
}))

vi.mock('@/lib/client', () => ({
  client: mockClient,
  isDesktop: () => false,
}))

streamAsksThroughAsk(mockClient)

const docker: HelpTopic = {
  id: 'install-docker',
  links: [
    { target: 'settings:computer', label: 'thisComputer' },
    {
      target: 'url:https://docs.docker.com/get-started/get-docker/',
      label: 'getDocker',
    },
  ],
}
const model: HelpTopic = {
  id: 'connect-model',
  links: [{ target: 'settings:models', label: 'models' }],
}

const helpAnswer = (overrides: Partial<HelpAnswer> = {}): HelpAnswer => ({
  topic: docker,
  related: [model],
  reason: 'noModel',
  detail: null,
  ...overrides,
})

const status = (effectiveModel: string | null) => ({
  status: 'ok',
  data: {
    defaultModel: 'claude:claude-sonnet-5',
    effectiveModel,
    claudeSignIn: null,
    trial: null,
  },
})

describe('the built-in help guide', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    useNavigationStore.setState(useNavigationStore.getInitialState())
    useUIStore.setState({ helpOpen: false, helpTopicId: null })
    mockClient.getDefaultModel.mockResolvedValue(status(null))
  })

  it('answers in the ask box with a link to the right screen', async () => {
    const response: AskResponse = { kind: 'help', data: helpAnswer() }
    mockClient.ask.mockResolvedValue({ status: 'ok', data: response })
    const user = userEvent.setup()
    render(
      <AskBox
        context={{ currentView: 'home', focusedArtifact: null }}
        placeholder="Ask anything"
      />
    )
    await user.type(
      screen.getByRole('textbox'),
      'How do I install Docker?{Enter}'
    )

    expect(await screen.findByText('Install Docker')).toBeInTheDocument()
    expect(
      screen.getByText(/No model is connected, so this answer is from/)
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open This computer' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'computer',
    })
  })

  it('opens web pages outside the app and other topics in place', async () => {
    const response: AskResponse = {
      kind: 'help',
      data: helpAnswer({ reason: 'modelFailed', detail: 'Offline.' }),
    }
    mockClient.ask.mockResolvedValue({ status: 'ok', data: response })
    const user = userEvent.setup()
    render(
      <AskBox
        context={{ currentView: 'home', focusedArtifact: null }}
        placeholder="Ask anything"
      />
    )
    await user.type(screen.getByRole('textbox'), 'docker{Enter}')
    expect(await screen.findByTestId('help-detail')).toHaveTextContent(
      'Offline.'
    )
    await user.click(screen.getByRole('button', { name: /Download Docker/ }))
    expect(mockClient.openExternal).toHaveBeenCalledWith(
      'https://docs.docker.com/get-started/get-docker/'
    )
    await user.click(screen.getByRole('button', { name: 'Connect a model' }))
    expect(screen.getByTestId('help-topic-connect-model')).toBeInTheDocument()
  })

  it('offers every topic when nothing matches', async () => {
    const response: AskResponse = {
      kind: 'help',
      data: helpAnswer({ topic: null, related: [docker, model] }),
    }
    mockClient.ask.mockResolvedValue({ status: 'ok', data: response })
    const user = userEvent.setup()
    render(
      <AskBox
        context={{ currentView: 'home', focusedArtifact: null }}
        placeholder="Ask anything"
      />
    )
    await user.type(screen.getByRole('textbox'), 'bake a cake{Enter}')
    expect(
      await screen.findByText(/I can’t answer that without a model/)
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Install Docker' })).toBeVisible()
  })

  it('a chat sent with no model also gets the guide', async () => {
    mockClient.helpAnswer.mockResolvedValue({
      status: 'ok',
      data: helpAnswer(),
    })
    const onSend = vi.fn()
    const user = userEvent.setup()
    render(
      <AskBox
        context={{ currentView: 'agent:writer', focusedArtifact: null }}
        placeholder="Ask"
        chat={{ placeholder: 'Message', onSend }}
      />
    )
    await waitFor(() => expect(mockClient.getDefaultModel).toHaveBeenCalled())
    await user.type(screen.getByRole('textbox'), 'install docker{Enter}')
    expect(onSend).toHaveBeenCalledWith('install docker', [])
    expect(await screen.findByText('Install Docker')).toBeInTheDocument()
    expect(mockClient.helpAnswer).toHaveBeenCalledWith('install docker')
  })

  it('a chat with a model gets no guide', async () => {
    mockClient.getDefaultModel.mockResolvedValue(
      status('claude:claude-sonnet-5')
    )
    const onSend = vi.fn()
    const user = userEvent.setup()
    render(
      <AskBox
        context={{ currentView: 'agent:writer', focusedArtifact: null }}
        placeholder="Ask"
        chat={{ placeholder: 'Message', onSend }}
      />
    )
    await waitFor(() => expect(mockClient.getDefaultModel).toHaveBeenCalled())
    await user.type(screen.getByRole('textbox'), 'install docker{Enter}')
    expect(onSend).toHaveBeenCalled()
    expect(mockClient.helpAnswer).not.toHaveBeenCalled()
  })

  it('the help window lists every topic and opens one', async () => {
    mockClient.helpTopics.mockResolvedValue({
      status: 'ok',
      data: [docker, model],
    })
    const user = userEvent.setup()
    render(<HelpGuideDialog />)
    useUIStore.getState().openHelp('connect-model')
    expect(
      await screen.findByTestId('help-topic-connect-model')
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Install Docker' }))
    expect(screen.getByTestId('help-topic-install-docker')).toBeInTheDocument()
    // Following a link to a screen closes the window
    await user.click(screen.getByRole('button', { name: 'Open This computer' }))
    expect(useUIStore.getState().helpOpen).toBe(false)
  })
})
