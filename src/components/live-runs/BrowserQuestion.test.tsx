import { describe, expect, it, vi, beforeEach } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { useStopStore } from '@/store/stop-store'
import { BrowserQuestion } from './BrowserQuestion'

describe('BrowserQuestion', () => {
  beforeEach(() => {
    useStopStore.setState({ question: null })
    Object.assign(commands, {
      answerBrowserQuestion: vi
        .fn()
        .mockResolvedValue({ status: 'ok', data: true }),
    })
  })

  it('asks after a stop and sends the answer', async () => {
    const user = userEvent.setup()
    useStopStore.getState().ask({ id: 'q1', until: Date.now() + 120_000 })
    render(<BrowserQuestion />)
    expect(screen.getByText('Close the browser too?')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Keep it open' }))
    expect(commands.answerBrowserQuestion).toHaveBeenCalledWith('q1', false)
    expect(screen.queryByText('Close the browser too?')).toBeNull()
  })

  it('is gone once its time is up', () => {
    useStopStore.getState().ask({ id: 'q2', until: Date.now() - 1 })
    render(<BrowserQuestion />)
    expect(screen.queryByText('Close the browser too?')).toBeNull()
  })
})
