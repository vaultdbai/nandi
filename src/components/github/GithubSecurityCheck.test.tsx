import { describe, it, expect, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { GithubSecurityCheck } from './GithubSecurityCheck'

describe('GithubSecurityCheck', () => {
  it('checks on the button and shows what it found', async () => {
    vi.mocked(commands.checkGithubSecurity).mockResolvedValueOnce({
      status: 'ok',
      data: { checkedAt: Date.now(), findings: 2 },
    })
    render(<GithubSecurityCheck />)
    expect(await screen.findByText('Not checked yet.')).toBeInTheDocument()

    await userEvent.click(
      screen.getByRole('button', { name: 'Check GitHub security' })
    )
    expect(commands.checkGithubSecurity).toHaveBeenCalledOnce()
    expect(
      await screen.findByText(/2 things to fix, in your to-dos/)
    ).toBeInTheDocument()
  })

  it('shows why a check failed', async () => {
    vi.mocked(commands.checkGithubSecurity).mockResolvedValueOnce({
      status: 'error',
      error: 'Sign in to GitHub first',
    })
    render(<GithubSecurityCheck />)
    await userEvent.click(
      await screen.findByRole('button', { name: 'Check GitHub security' })
    )
    expect(
      await screen.findByText(/Sign in to GitHub first/)
    ).toBeInTheDocument()
  })
})
