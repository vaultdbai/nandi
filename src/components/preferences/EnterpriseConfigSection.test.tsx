import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { RepoLlmConfigStatus } from '@/lib/bindings'
import { EnterpriseConfigSection } from './EnterpriseConfigSection'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const loaded: RepoLlmConfigStatus = {
  found: true,
  path: '/home/me/.nandi/repo/.nandi/llm.yaml',
  config: {
    providers: [
      {
        id: 'bedrock-primary',
        kind: 'bedrock',
        region: 'us-east-1',
        profile: 'acme-dev',
        endpoint: null,
        model: null,
      },
      {
        id: 'gateway',
        kind: 'litellm',
        region: null,
        profile: null,
        endpoint: 'https://llm.acme.internal',
        model: null,
      },
    ],
    defaultProvider: 'bedrock-primary',
    fallbacks: ['gateway'],
  },
  error: null,
  effectiveProvider: { value: 'bedrock', source: 'repo' },
  effectiveRegion: { value: 'eu-west-1', source: 'environment' },
  effectiveProfile: { value: 'acme-dev', source: 'repo' },
  effectiveModel: { value: 'claude:claude-sonnet-5', source: 'local' },
  syncedAt: null,
}

describe('EnterpriseConfigSection', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getRepoLlmConfig: ok(loaded),
      reloadRepoLlmConfig: ok({ ...loaded, syncedAt: Date.now() }),
    })
  })

  it("shows the repo's providers and where each effective value comes from", async () => {
    render(<EnterpriseConfigSection />)
    const providers = await screen.findByTestId('enterprise-providers')
    expect(providers).toHaveTextContent('bedrock-primary')
    expect(providers).toHaveTextContent('Default')
    expect(providers).toHaveTextContent('Fallback')
    expect(providers).toHaveTextContent('https://llm.acme.internal')
    expect(screen.getByText('eu-west-1')).toBeInTheDocument()
    expect(screen.getByText('environment')).toBeInTheDocument()
    expect(screen.getByText('this machine')).toBeInTheDocument()
  })

  it('syncs the file on demand', async () => {
    const user = userEvent.setup()
    render(<EnterpriseConfigSection />)
    await screen.findByTestId('enterprise-providers')
    await user.click(screen.getByRole('button', { name: 'Sync' }))
    await waitFor(() => expect(commands.reloadRepoLlmConfig).toHaveBeenCalled())
    expect(await screen.findByText(/^Synced/)).toBeInTheDocument()
  })

  it('says when the repo has no config, and shows a broken one', async () => {
    Object.assign(commands, {
      getRepoLlmConfig: ok({
        ...loaded,
        found: false,
        config: null,
        effectiveProvider: null,
      }),
    })
    const { unmount } = render(<EnterpriseConfigSection />)
    expect(await screen.findByTestId('enterprise-none')).toHaveTextContent(
      '.nandi/llm.yaml'
    )
    unmount()

    Object.assign(commands, {
      getRepoLlmConfig: ok({
        ...loaded,
        config: null,
        error: '.nandi/llm.yaml lists no providers',
      }),
    })
    render(<EnterpriseConfigSection />)
    expect(
      await screen.findByText('.nandi/llm.yaml lists no providers')
    ).toBeInTheDocument()
  })
})
