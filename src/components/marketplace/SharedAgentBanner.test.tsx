import { describe, it, expect, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { agentInfo } from '@/test/fixtures'
import { commands } from '@/lib/tauri-bindings'
import type { ConsentStatus } from '@/lib/bindings'
import { SharedAgentBanner } from './SharedAgentBanner'

describe('SharedAgentBanner', () => {
  const consent: ConsentStatus = {
    agentId: 'arxiv-researcher',
    agentName: 'ArXiv Researcher',
    required: true,
    satisfied: false,
    categories: [
      { key: 'tools', items: ['shell', 'read_file'], granted: false },
      { key: 'network', items: ['export.arxiv.org'], granted: false },
    ],
  }

  it('shows nothing for the user’s own agents', () => {
    const { container } = render(
      <SharedAgentBanner
        agent={agentInfo('mine', 'Mine')}
        onOpenRules={vi.fn()}
      />
    )
    expect(container).toBeEmptyDOMElement()
  })

  it('sends the user to Your rules, and offers a fork for the rest', async () => {
    const onOpenRules = vi.fn()
    render(
      <SharedAgentBanner
        agent={agentInfo('writer', 'Writer', {
          origin: { kind: 'pack', pack: 'Nandi Dev Pack' },
        })}
        onOpenRules={onOpenRules}
      />
    )
    expect(
      screen.getByText(
        'This is a shared agent. Change its rules in Settings › Your rules; fork it to change how it works.'
      )
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Fork' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Your rules' }))
    expect(onOpenRules).toHaveBeenCalledOnce()
  })

  it('asks for approval of every category before the first run', async () => {
    vi.mocked(commands.getAgentConsent).mockResolvedValue({
      status: 'ok',
      data: consent,
    })
    vi.mocked(commands.grantAgentConsent).mockResolvedValueOnce({
      status: 'ok',
      data: {
        ...consent,
        satisfied: true,
        categories: consent.categories.map(c => ({ ...c, granted: true })),
      },
    })
    render(
      <SharedAgentBanner
        agent={agentInfo('arxiv-researcher', 'ArXiv Researcher', {
          origin: { kind: 'publisher', publisher: 'alice' },
        })}
        onOpenRules={vi.fn()}
      />
    )
    await userEvent.click(
      await screen.findByRole('button', { name: 'Review and allow' })
    )

    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('export.arxiv.org')).toBeInTheDocument()
    const allow = within(dialog).getByRole('button', { name: 'Allow' })
    expect(allow).toBeDisabled()
    await userEvent.click(within(dialog).getByLabelText(/Tools/))
    expect(allow).toBeDisabled()
    await userEvent.click(within(dialog).getByLabelText(/Network hosts/))
    await userEvent.click(allow)
    expect(commands.grantAgentConsent).toHaveBeenCalledWith(
      'arxiv-researcher',
      [
        { key: 'tools', items: ['shell', 'read_file'] },
        { key: 'network', items: ['export.arxiv.org'] },
      ]
    )
    expect(
      screen.queryByRole('button', { name: 'Review and allow' })
    ).not.toBeInTheDocument()
  })
})
